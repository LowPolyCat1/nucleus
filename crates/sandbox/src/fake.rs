//! An in-process [`SandboxBackend`] for tests, enabled with the `fake` feature.
//!
//! Containers are emulated: processes run on the host, and container paths are translated to
//! host paths through the container's bind mounts (longest prefix wins). That is enough to test
//! everything above the sandbox (conversation lifecycle, turns, commits, proposals) without a
//! container engine. It provides no isolation and does not enforce read-only mounts or network
//! policies; never use it outside tests.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Mutex;

use anyhow::{anyhow, bail};
use async_trait::async_trait;
use futures::{FutureExt, StreamExt};
use tokio::io::AsyncReadExt;

use crate::{ContainerInfo, ContainerSpec, Engine, ExecChunk, ExecHandle, ExecSpec, MountMode, Result, SandboxBackend};

#[derive(Debug, Clone)]
struct FakeContainer {
    spec: ContainerSpec,
    volumes: Vec<(String, PathBuf)>,
}

/// A recorded exec call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecRecord {
    pub container: String,
    pub cmd: Vec<String>,
    pub workdir: Option<String>,
}

pub struct FakeBackend {
    engine: Engine,
    /// Host directories prepended to `PATH` for every exec, e.g. holding a fake `claude`.
    bin_dirs: Vec<PathBuf>,
    volume_root: PathBuf,
    containers: Mutex<HashMap<String, FakeContainer>>,
    execs: Mutex<Vec<ExecRecord>>,
    fail_create: Mutex<Option<String>>,
}

impl FakeBackend {
    pub fn new(engine: Engine, volume_root: impl Into<PathBuf>) -> Self {
        Self {
            engine,
            bin_dirs: Vec::new(),
            volume_root: volume_root.into(),
            containers: Mutex::new(HashMap::new()),
            execs: Mutex::new(Vec::new()),
            fail_create: Mutex::new(None),
        }
    }

    pub fn with_bin_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.bin_dirs.push(dir.into());
        self
    }

    /// Make the next `create` fail with this message.
    pub fn fail_next_create(&self, message: &str) {
        *self.fail_create.lock().unwrap() = Some(message.to_string());
    }

    pub fn spec(&self, name: &str) -> Option<ContainerSpec> {
        self.containers.lock().unwrap().get(name).map(|c| c.spec.clone())
    }

    pub fn container_names(&self) -> Vec<String> {
        let mut v: Vec<_> = self.containers.lock().unwrap().keys().cloned().collect();
        v.sort();
        v
    }

    pub fn execs(&self) -> Vec<ExecRecord> {
        self.execs.lock().unwrap().clone()
    }

    /// Translate a container path to a host path through the container's mounts.
    pub fn host_path(&self, container: &str, path: &str) -> Option<PathBuf> {
        let containers = self.containers.lock().unwrap();
        let c = containers.get(container)?;
        map_path(c, path)
    }
}

fn mounts(c: &FakeContainer) -> Vec<(String, PathBuf)> {
    let mut m: Vec<(String, PathBuf)> = c
        .spec
        .binds
        .iter()
        .map(|b| (b.target.clone(), b.source.clone()))
        .collect();
    m.extend(c.volumes.iter().cloned());
    m.sort_by_key(|(t, _)| std::cmp::Reverse(t.len()));
    m
}

fn map_path(c: &FakeContainer, path: &str) -> Option<PathBuf> {
    for (target, source) in mounts(c) {
        if path == target {
            return Some(source);
        }
        if let Some(rest) = path.strip_prefix(&format!("{target}/")) {
            return Some(source.join(rest));
        }
    }
    None
}

/// Rewrite every container path inside `s` (also inside `:`-separated lists and longer strings).
fn map_str(c: &FakeContainer, s: &str) -> String {
    let mut out = s.to_string();
    for (target, source) in mounts(c) {
        let src = source.to_string_lossy();
        let mut result = String::new();
        let mut rest = out.as_str();
        while let Some(pos) = rest.find(target.as_str()) {
            let end = pos + target.len();
            let boundary_before = pos == 0
                || matches!(
                    rest.as_bytes()[pos - 1],
                    b':' | b' ' | b'=' | b'"' | b'\'' | b'\n' | b';' | b'('
                );
            let boundary_after =
                end == rest.len() || matches!(rest.as_bytes()[end], b'/' | b':' | b' ' | b'"' | b'\'' | b'\n' | b';');
            if boundary_before && boundary_after {
                result.push_str(&rest[..pos]);
                result.push_str(&src);
            } else {
                result.push_str(&rest[..end]);
            }
            rest = &rest[end..];
        }
        result.push_str(rest);
        out = result;
    }
    out
}

#[async_trait]
impl SandboxBackend for FakeBackend {
    fn engine(&self) -> Engine {
        self.engine
    }

    async fn ensure_image(&self, image: &str) -> Result<String> {
        if image.is_empty() {
            bail!("empty image name");
        }
        Ok(format!("sha256:fake-{}", image.replace(['/', ':'], "-")))
    }

    async fn image_env(&self, image: &str) -> Result<Vec<String>> {
        self.ensure_image(image).await?;
        Ok(vec![format!("PATH={}", std::env::var("PATH").unwrap_or_default())])
    }

    async fn create(&self, spec: &ContainerSpec) -> Result<ContainerInfo> {
        if let Some(msg) = self.fail_create.lock().unwrap().take() {
            bail!("{msg}");
        }
        if spec.binds.iter().any(|b| b.mode == MountMode::Overlay) && !self.engine.supports_overlay() {
            bail!("overlay mounts require Podman");
        }
        for b in &spec.binds {
            if !b.source.exists() {
                bail!("bind source {} does not exist", b.source.display());
            }
        }
        let mut containers = self.containers.lock().unwrap();
        if containers.contains_key(&spec.name) {
            bail!("container {} already exists", spec.name);
        }
        let mut volumes = Vec::new();
        for v in &spec.volumes {
            let dir = self.volume_root.join(&v.volume);
            std::fs::create_dir_all(&dir)?;
            volumes.push((v.target.clone(), dir));
        }
        containers.insert(
            spec.name.clone(),
            FakeContainer {
                spec: spec.clone(),
                volumes,
            },
        );
        Ok(ContainerInfo {
            id: format!("fake-{}", spec.name),
            name: spec.name.clone(),
            labels: spec.labels.clone(),
            running: true,
        })
    }

    async fn exec(&self, container: &str, spec: &ExecSpec) -> Result<ExecHandle> {
        let c = self
            .containers
            .lock()
            .unwrap()
            .get(container)
            .cloned()
            .ok_or_else(|| anyhow!("no such container: {container}"))?;
        self.execs.lock().unwrap().push(ExecRecord {
            container: container.into(),
            cmd: spec.cmd.clone(),
            workdir: spec.workdir.clone(),
        });
        let workdir = spec.workdir.clone().or(c.spec.workdir.clone());
        let mut env: BTreeMap<String, String> = c.spec.env.clone();
        env.extend(spec.env.clone());
        let mut path: Vec<String> = self.bin_dirs.iter().map(|d| d.to_string_lossy().to_string()).collect();
        if let Some(p) = env.remove("PATH") {
            path.push(map_str(&c, &p));
        }
        path.push(std::env::var("PATH").unwrap_or_default());
        let (prog, args) = spec.cmd.split_first().ok_or_else(|| anyhow!("empty command"))?;
        let mut cmd = tokio::process::Command::new(map_str(&c, prog));
        cmd.args(args.iter().map(|a| map_str(&c, a)));
        for (k, v) in &env {
            cmd.env(k, map_str(&c, v));
        }
        cmd.env("PATH", path.join(":"));
        if let Some(w) = workdir {
            let host = map_path(&c, &w).ok_or_else(|| anyhow!("workdir {w} is not mounted in the fake container"))?;
            cmd.current_dir(host);
        }
        cmd.stdin(if spec.stdin {
            std::process::Stdio::piped()
        } else {
            std::process::Stdio::null()
        })
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
        let mut child = cmd.spawn()?;
        let stdin = child
            .stdin
            .take()
            .map(|s| Box::pin(s) as std::pin::Pin<Box<dyn tokio::io::AsyncWrite + Send>>);
        let (tx, rx) = futures::channel::mpsc::unbounded();
        let pipes: Vec<(Box<dyn tokio::io::AsyncRead + Send + Unpin>, bool)> = vec![
            (Box::new(child.stdout.take().unwrap()), false),
            (Box::new(child.stderr.take().unwrap()), true),
        ];
        let mut readers = Vec::new();
        for (mut pipe, is_err) in pipes {
            let tx = tx.clone();
            readers.push(tokio::spawn(async move {
                let mut buf = vec![0u8; 8192];
                while let Ok(n) = pipe.read(&mut buf).await {
                    if n == 0 {
                        break;
                    }
                    let chunk = buf[..n].to_vec();
                    let _ = tx.unbounded_send(Ok(if is_err {
                        ExecChunk::Stderr(chunk)
                    } else {
                        ExecChunk::Stdout(chunk)
                    }));
                }
            }));
        }
        drop(tx);
        let exit = async move {
            for r in readers {
                r.await.ok();
            }
            Ok(child.wait().await?.code().map(i64::from))
        }
        .boxed();
        Ok(ExecHandle::new(rx.boxed(), stdin, exit))
    }

    async fn remove(&self, name: &str) -> Result<()> {
        self.containers.lock().unwrap().remove(name);
        Ok(())
    }

    async fn list(&self, label: Option<(&str, &str)>) -> Result<Vec<ContainerInfo>> {
        Ok(self
            .containers
            .lock()
            .unwrap()
            .values()
            .filter(|c| label.is_none_or(|(k, v)| c.spec.labels.get(k).map(String::as_str) == Some(v)))
            .map(|c| ContainerInfo {
                id: format!("fake-{}", c.spec.name),
                name: c.spec.name.clone(),
                labels: c.spec.labels.clone(),
                running: true,
            })
            .collect())
    }

    async fn ensure_volume(&self, name: &str) -> Result<()> {
        std::fs::create_dir_all(self.volume_root.join(name))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BindMount;

    #[tokio::test]
    async fn maps_paths_and_runs() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join("ws");
        std::fs::create_dir_all(&ws).unwrap();
        let b = FakeBackend::new(Engine::Docker, dir.path().join("vols"));
        let mut spec = ContainerSpec::new("c", "img");
        spec.binds.push(BindMount {
            source: ws.clone(),
            target: "/workspace".into(),
            mode: MountMode::ReadWrite,
        });
        spec.workdir = Some("/workspace".into());
        spec.env.insert("OUT".into(), "/workspace/out.txt".into());
        b.create(&spec).await.unwrap();
        assert!(b.create(&spec).await.is_err(), "duplicate names are rejected");
        let r = b
            .exec_collect("c", &ExecSpec::new(["sh", "-c", "pwd > $OUT; echo hi"]))
            .await
            .unwrap();
        assert_eq!(r.stdout_str(), "hi\n");
        assert_eq!(
            std::fs::read_to_string(ws.join("out.txt")).unwrap().trim(),
            ws.to_str().unwrap()
        );
        assert_eq!(b.host_path("c", "/workspace/a/b"), Some(ws.join("a/b")));
        assert_eq!(b.host_path("c", "/workspacex"), None);
        // Prefix that is not a path boundary is left alone.
        let c = b.containers.lock().unwrap().get("c").cloned().unwrap();
        assert_eq!(map_str(&c, "/workspaces"), "/workspaces");
        assert_eq!(
            map_str(&c, "x:/workspace/bin:/usr/bin"),
            format!("x:{}/bin:/usr/bin", ws.display())
        );
        b.remove("c").await.unwrap();
        assert!(b.exec("c", &ExecSpec::new(["true"])).await.is_err());

        let mut o = ContainerSpec::new("o", "img");
        o.binds.push(BindMount {
            source: ws.clone(),
            target: "/deps/x".into(),
            mode: MountMode::Overlay,
        });
        assert!(b.create(&o).await.is_err(), "docker has no overlay");
    }
}
