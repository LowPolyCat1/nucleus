use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use anyhow::{Context, anyhow, bail};
use async_trait::async_trait;
use bollard::Docker;
use bollard::exec::{CreateExecOptions, StartExecOptions, StartExecResults};
use bollard::models::{
    ContainerCreateBody, EndpointSettings, HostConfig, NetworkConnectRequest, NetworkCreateRequest, VolumeCreateRequest,
};
use bollard::query_parameters::{
    CreateContainerOptions, CreateImageOptions, ListContainersOptions, RemoveContainerOptions,
};
use futures::{FutureExt, StreamExt, TryStreamExt};

use crate::network::EgressPlan;
use crate::{
    ContainerInfo, ContainerSpec, Engine, ExecChunk, ExecHandle, ExecSpec, MANAGED_LABEL, MountMode, Result,
    SandboxBackend,
};

const EGRESS_JS: &str = include_str!("../support/egress.js");
const EGRESS_PORT: u16 = 3128;
const EGRESS_ALIAS: &str = "egress";

/// Find the container engine socket. Order: `NUCLEUS_CONTAINER_SOCKET`, rootless Podman,
/// rootful Podman, Docker.
pub fn detect_socket() -> Option<PathBuf> {
    if let Ok(s) = std::env::var("NUCLEUS_CONTAINER_SOCKET") {
        return Some(PathBuf::from(s.trim_start_matches("unix://")));
    }
    let mut candidates = Vec::new();
    if let Ok(rt) = std::env::var("XDG_RUNTIME_DIR") {
        candidates.push(PathBuf::from(rt).join("podman/podman.sock"));
    }
    candidates.push("/run/podman/podman.sock".into());
    candidates.push("/var/run/docker.sock".into());
    if let Ok(home) = std::env::var("HOME") {
        candidates.push(PathBuf::from(home).join(".docker/run/docker.sock"));
    }
    candidates.into_iter().find(|p| p.exists())
}

/// [`SandboxBackend`] for Podman and Docker through the Docker-compatible API.
#[derive(Clone)]
pub struct BollardBackend {
    docker: Docker,
    engine: Engine,
    /// Host directory for support files mounted into containers (the egress proxy script).
    support_dir: PathBuf,
}

impl BollardBackend {
    pub async fn connect(socket: &Path, support_dir: impl Into<PathBuf>) -> Result<Self> {
        let docker = Docker::connect_with_unix(
            socket.to_str().ok_or_else(|| anyhow!("non utf-8 socket path"))?,
            300,
            bollard::API_DEFAULT_VERSION,
        )?;
        let version = docker.version().await.context("container engine is not reachable")?;
        let is_podman = version
            .components
            .unwrap_or_default()
            .iter()
            .any(|c| c.name.to_ascii_lowercase().contains("podman"));
        let support_dir = support_dir.into();
        std::fs::create_dir_all(&support_dir)?;
        std::fs::write(support_dir.join("egress.js"), EGRESS_JS)?;
        Ok(Self {
            docker,
            engine: if is_podman { Engine::Podman } else { Engine::Docker },
            support_dir,
        })
    }

    /// Connect to the auto-detected socket.
    pub async fn connect_default(support_dir: impl Into<PathBuf>) -> Result<Self> {
        let socket = detect_socket().ok_or_else(|| {
            anyhow!("no Podman or Docker socket found; start `podman system service` or Docker, or set NUCLEUS_CONTAINER_SOCKET")
        })?;
        Self::connect(&socket, support_dir).await
    }

    pub fn docker(&self) -> &Docker {
        &self.docker
    }

    fn network_name(container: &str) -> String {
        format!("{container}-net")
    }

    fn egress_name(container: &str) -> String {
        format!("{container}-egress")
    }

    fn binds(&self, spec: &ContainerSpec) -> Result<Vec<String>> {
        let mut binds = Vec::new();
        for b in &spec.binds {
            let src = b.source.to_str().ok_or_else(|| anyhow!("non utf-8 mount source"))?;
            let opt = match b.mode {
                MountMode::ReadOnly => "ro",
                MountMode::ReadWrite => "rw",
                MountMode::Overlay if self.engine.supports_overlay() => "O",
                MountMode::Overlay => bail!(
                    "overlay mount for {} requires Podman; the Docker engine has no equivalent",
                    b.target
                ),
            };
            let selinux = if self.engine == Engine::Podman && b.mode != MountMode::Overlay {
                ",z"
            } else {
                ""
            };
            binds.push(format!("{src}:{}:{opt}{selinux}", b.target));
        }
        for v in &spec.volumes {
            binds.push(format!("{}:{}", v.volume, v.target));
        }
        Ok(binds)
    }

    fn labels(spec_labels: &BTreeMap<String, String>) -> HashMap<String, String> {
        let mut labels: HashMap<_, _> = spec_labels.clone().into_iter().collect();
        labels.insert(MANAGED_LABEL.into(), "true".into());
        labels
    }

    async fn create_and_start(&self, name: &str, body: ContainerCreateBody) -> Result<String> {
        let created = self
            .docker
            .create_container(
                Some(CreateContainerOptions {
                    name: Some(name.into()),
                    ..Default::default()
                }),
                body,
            )
            .await
            .with_context(|| format!("creating container {name}"))?;
        self.docker
            .start_container(name, None)
            .await
            .with_context(|| format!("starting container {name}"))?;
        Ok(created.id)
    }

    async fn start_egress(&self, spec: &ContainerSpec, allow: &[String]) -> Result<String> {
        let net = Self::network_name(&spec.name);
        self.docker
            .create_network(NetworkCreateRequest {
                name: net.clone(),
                internal: Some(true),
                labels: Some(Self::labels(&spec.labels)),
                ..Default::default()
            })
            .await
            .with_context(|| format!("creating network {net}"))?;
        let egress = Self::egress_name(&spec.name);
        let script = self.support_dir.join("egress.js");
        let mut labels = Self::labels(&spec.labels);
        labels.insert("nucleus.role".into(), "egress".into());
        let body = ContainerCreateBody {
            image: Some(spec.image.clone()),
            cmd: Some(vec!["node".into(), "/nucleus/egress.js".into()]),
            entrypoint: Some(vec![]),
            env: Some(vec![
                format!("NUCLEUS_ALLOW={}", allow.join(",")),
                format!("NUCLEUS_EGRESS_PORT={EGRESS_PORT}"),
            ]),
            labels: Some(labels),
            host_config: Some(HostConfig {
                binds: Some(vec![format!("{}:/nucleus/egress.js:ro", script.display())]),
                cap_drop: Some(vec!["ALL".into()]),
                security_opt: Some(vec!["no-new-privileges".into()]),
                memory: Some(256 << 20),
                ..Default::default()
            }),
            ..Default::default()
        };
        self.create_and_start(&egress, body).await?;
        self.docker
            .connect_network(
                &net,
                NetworkConnectRequest {
                    container: egress.clone(),
                    endpoint_config: Some(EndpointSettings {
                        aliases: Some(vec![EGRESS_ALIAS.into()]),
                        ..Default::default()
                    }),
                },
            )
            .await
            .context("attaching egress proxy to sandbox network")?;
        Ok(net)
    }

    async fn remove_container(&self, name: &str) -> Result<()> {
        match self
            .docker
            .remove_container(
                name,
                Some(RemoveContainerOptions {
                    force: true,
                    v: false,
                    link: false,
                }),
            )
            .await
        {
            Ok(()) => Ok(()),
            Err(bollard::errors::Error::DockerResponseServerError { status_code: 404, .. }) => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}

#[async_trait]
impl SandboxBackend for BollardBackend {
    fn engine(&self) -> Engine {
        self.engine
    }

    async fn ensure_image(&self, image: &str) -> Result<String> {
        if let Ok(info) = self.docker.inspect_image(image).await {
            return info.id.ok_or_else(|| anyhow!("image {image} has no id"));
        }
        let mut pull = self.docker.create_image(
            Some(CreateImageOptions {
                from_image: Some(image.into()),
                ..Default::default()
            }),
            None,
            None,
        );
        while let Some(progress) = pull.next().await {
            progress.with_context(|| format!("pulling {image}"))?;
        }
        let info = self.docker.inspect_image(image).await?;
        info.id.ok_or_else(|| anyhow!("image {image} has no id"))
    }

    async fn image_env(&self, image: &str) -> Result<Vec<String>> {
        self.ensure_image(image).await?;
        let info = self.docker.inspect_image(image).await?;
        Ok(info.config.and_then(|c| c.env).unwrap_or_default())
    }

    async fn create(&self, spec: &ContainerSpec) -> Result<ContainerInfo> {
        self.ensure_image(&spec.image).await?;
        for v in &spec.volumes {
            self.ensure_volume(&v.volume).await?;
        }
        let plan = EgressPlan::for_policy(&spec.network, &spec.required_hosts);
        let mut env: BTreeMap<String, String> = spec.env.clone();
        let network_mode = match &plan {
            EgressPlan::Isolated => Some("none".to_string()),
            EgressPlan::Open => None,
            EgressPlan::Proxied { allow } => {
                let net = self.start_egress(spec, allow).await?;
                let proxy = format!("http://{EGRESS_ALIAS}:{EGRESS_PORT}");
                for k in ["HTTPS_PROXY", "HTTP_PROXY", "https_proxy", "http_proxy"] {
                    env.insert(k.into(), proxy.clone());
                }
                env.insert("NO_PROXY".into(), "localhost,127.0.0.1".into());
                env.insert("no_proxy".into(), "localhost,127.0.0.1".into());
                Some(net)
            }
        };
        let (user, userns_mode) = match self.engine {
            // keep-id maps the host user into the container so files in the worktree keep their owner.
            Engine::Podman => (spec.user.clone(), Some("keep-id".to_string())),
            Engine::Docker => (spec.user.clone(), None),
        };
        let body = ContainerCreateBody {
            image: Some(spec.image.clone()),
            // Keep the container alive; work happens through exec.
            entrypoint: Some(vec!["sleep".into()]),
            cmd: Some(vec!["infinity".into()]),
            env: Some(env.iter().map(|(k, v)| format!("{k}={v}")).collect()),
            working_dir: spec.workdir.clone(),
            user,
            labels: Some(Self::labels(&spec.labels)),
            host_config: Some(HostConfig {
                binds: Some(self.binds(spec)?),
                network_mode,
                userns_mode,
                init: Some(true),
                cap_drop: Some(vec!["ALL".into()]),
                cap_add: Some(vec![
                    "CHOWN".into(),
                    "DAC_OVERRIDE".into(),
                    "FOWNER".into(),
                    "SETUID".into(),
                    "SETGID".into(),
                ]),
                security_opt: Some(vec!["no-new-privileges".into()]),
                memory: spec.limits.memory_bytes,
                nano_cpus: spec.limits.nano_cpus,
                pids_limit: spec.limits.pids,
                ..Default::default()
            }),
            ..Default::default()
        };
        let id = match self.create_and_start(&spec.name, body).await {
            Ok(id) => id,
            Err(e) => {
                self.remove(&spec.name).await.ok();
                return Err(e);
            }
        };
        if !spec.chown_paths.is_empty()
            && let Some(user) = &spec.user
        {
            let mut cmd = vec!["chown".to_string(), user.clone()];
            cmd.extend(spec.chown_paths.iter().cloned());
            let res = self
                .exec_collect(
                    &spec.name,
                    &ExecSpec {
                        user: Some("0:0".into()),
                        ..ExecSpec::new(cmd)
                    },
                )
                .await?;
            if !res.success() {
                tracing::warn!(container = %spec.name, stderr = %res.stderr_str(), "chown of cache volumes failed");
            }
        }
        Ok(ContainerInfo {
            id,
            name: spec.name.clone(),
            labels: spec.labels.clone(),
            running: true,
        })
    }

    async fn exec(&self, container: &str, spec: &ExecSpec) -> Result<ExecHandle> {
        let exec = self
            .docker
            .create_exec(
                container,
                CreateExecOptions::<String> {
                    attach_stdin: Some(spec.stdin),
                    attach_stdout: Some(true),
                    attach_stderr: Some(true),
                    tty: Some(false),
                    env: Some(spec.env.iter().map(|(k, v)| format!("{k}={v}")).collect()),
                    cmd: Some(spec.cmd.clone()),
                    user: spec.user.clone(),
                    working_dir: spec.workdir.clone(),
                    ..Default::default()
                },
            )
            .await
            .with_context(|| format!("exec in {container}"))?;
        let StartExecResults::Attached { output, input } = self
            .docker
            .start_exec(
                &exec.id,
                Some(StartExecOptions {
                    detach: false,
                    tty: false,
                    output_capacity: Some(64 * 1024),
                }),
            )
            .await?
        else {
            bail!("exec unexpectedly detached");
        };
        use bollard::container::LogOutput;
        let output = output
            .map_err(anyhow::Error::from)
            .try_filter_map(|chunk| async move {
                Ok(match chunk {
                    LogOutput::StdOut { message } | LogOutput::Console { message } => {
                        Some(ExecChunk::Stdout(message.to_vec()))
                    }
                    LogOutput::StdErr { message } => Some(ExecChunk::Stderr(message.to_vec())),
                    LogOutput::StdIn { .. } => None,
                })
            })
            .boxed();
        let docker = self.docker.clone();
        let id = exec.id.clone();
        let exit = async move {
            // The exit code can lag behind the end of the output stream for a moment.
            for _ in 0..50 {
                let info = docker.inspect_exec(&id).await?;
                if info.running != Some(true) {
                    return Ok(info.exit_code);
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
            Ok(None)
        }
        .boxed();
        Ok(ExecHandle::new(output, spec.stdin.then_some(input), exit))
    }

    async fn remove(&self, name: &str) -> Result<()> {
        self.remove_container(name).await?;
        self.remove_container(&Self::egress_name(name)).await?;
        match self.docker.remove_network(&Self::network_name(name)).await {
            Ok(()) | Err(bollard::errors::Error::DockerResponseServerError { status_code: 404, .. }) => Ok(()),
            Err(e) => Err(e.into()),
        }
    }

    async fn list(&self, label: Option<(&str, &str)>) -> Result<Vec<ContainerInfo>> {
        let mut filters = vec![format!("{MANAGED_LABEL}=true")];
        if let Some((k, v)) = label {
            filters.push(format!("{k}={v}"));
        }
        let list = self
            .docker
            .list_containers(Some(ListContainersOptions {
                all: true,
                filters: Some(HashMap::from([("label".to_string(), filters)])),
                ..Default::default()
            }))
            .await?;
        Ok(list
            .into_iter()
            .filter(|c| c.labels.as_ref().and_then(|l| l.get("nucleus.role")).is_none())
            .map(|c| ContainerInfo {
                id: c.id.unwrap_or_default(),
                name: c
                    .names
                    .unwrap_or_default()
                    .first()
                    .map(|n| n.trim_start_matches('/').to_string())
                    .unwrap_or_default(),
                labels: c.labels.unwrap_or_default().into_iter().collect(),
                running: c.state.map(|s| s.to_string() == "running").unwrap_or(false),
            })
            .collect())
    }

    async fn ensure_volume(&self, name: &str) -> Result<()> {
        if self.docker.inspect_volume(name).await.is_ok() {
            return Ok(());
        }
        self.docker
            .create_volume(VolumeCreateRequest {
                name: Some(name.into()),
                labels: Some(HashMap::from([(MANAGED_LABEL.to_string(), "true".to_string())])),
                ..Default::default()
            })
            .await?;
        Ok(())
    }
}
