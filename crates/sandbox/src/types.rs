use std::collections::BTreeMap;
use std::path::PathBuf;
use std::pin::Pin;

use futures::future::BoxFuture;
use futures::stream::BoxStream;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWrite;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Engine {
    Podman,
    Docker,
}

impl Engine {
    pub fn supports_overlay(self) -> bool {
        matches!(self, Engine::Podman)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MountMode {
    ReadOnly,
    ReadWrite,
    /// Podman `:O` overlay: writable inside the container, writes discarded on exit.
    Overlay,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BindMount {
    pub source: PathBuf,
    pub target: String,
    pub mode: MountMode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VolumeMount {
    pub volume: String,
    pub target: String,
}

/// Outbound network access for a container.
///
/// The model provider's hosts (`ContainerSpec::required_hosts`) are always reachable, because
/// the agent CLI runs inside the container. Everything else follows the policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "mode", content = "hosts", rename_all = "lowercase")]
pub enum NetworkPolicy {
    /// No egress beyond the required hosts. The default.
    #[default]
    None,
    /// Required hosts plus these. `*.example.com` matches subdomains.
    Allowlist(Vec<String>),
    /// Unrestricted access.
    Full,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerSpec {
    pub name: String,
    pub image: String,
    pub binds: Vec<BindMount>,
    pub volumes: Vec<VolumeMount>,
    pub env: BTreeMap<String, String>,
    pub workdir: Option<String>,
    /// `uid:gid` to run as. On Podman the harness maps the host user with `keep-id` instead.
    pub user: Option<String>,
    pub network: NetworkPolicy,
    /// Hosts that must be reachable regardless of policy (the LLM provider).
    pub required_hosts: Vec<String>,
    pub labels: BTreeMap<String, String>,
    /// Paths (typically volume targets) to chown to `user` after start, so a non-root user can
    /// write to freshly created named volumes.
    pub chown_paths: Vec<String>,
    pub limits: Limits,
}

impl ContainerSpec {
    pub fn new(name: impl Into<String>, image: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            image: image.into(),
            binds: Vec::new(),
            volumes: Vec::new(),
            env: BTreeMap::new(),
            workdir: None,
            user: None,
            network: NetworkPolicy::None,
            required_hosts: Vec::new(),
            labels: BTreeMap::new(),
            chown_paths: Vec::new(),
            limits: Limits::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Limits {
    pub memory_bytes: Option<i64>,
    pub nano_cpus: Option<i64>,
    pub pids: Option<i64>,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            memory_bytes: Some(8 << 30),
            nano_cpus: None,
            pids: Some(4096),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerInfo {
    pub id: String,
    pub name: String,
    pub labels: BTreeMap<String, String>,
    pub running: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecSpec {
    pub cmd: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub workdir: Option<String>,
    pub user: Option<String>,
    /// Keep stdin open; write to [`ExecHandle::stdin`].
    pub stdin: bool,
}

impl ExecSpec {
    pub fn new<I, S>(cmd: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            cmd: cmd.into_iter().map(Into::into).collect(),
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecChunk {
    Stdout(Vec<u8>),
    Stderr(Vec<u8>),
}

/// A running process. Drain `output` before awaiting [`ExecHandle::wait`].
pub struct ExecHandle {
    pub output: BoxStream<'static, crate::Result<ExecChunk>>,
    pub stdin: Option<Pin<Box<dyn AsyncWrite + Send>>>,
    pub(crate) exit: BoxFuture<'static, crate::Result<Option<i64>>>,
}

impl ExecHandle {
    pub fn new(
        output: BoxStream<'static, crate::Result<ExecChunk>>,
        stdin: Option<Pin<Box<dyn AsyncWrite + Send>>>,
        exit: BoxFuture<'static, crate::Result<Option<i64>>>,
    ) -> Self {
        Self { output, stdin, exit }
    }

    /// Split into output stream, stdin and the exit future.
    #[allow(clippy::type_complexity)]
    pub fn into_parts(
        self,
    ) -> (
        BoxStream<'static, crate::Result<ExecChunk>>,
        Option<Pin<Box<dyn AsyncWrite + Send>>>,
        BoxFuture<'static, crate::Result<Option<i64>>>,
    ) {
        (self.output, self.stdin, self.exit)
    }

    /// Exit code of the process, once its output has been fully read.
    pub async fn wait(self) -> crate::Result<Option<i64>> {
        self.exit.await
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecResult {
    pub exit_code: Option<i64>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl ExecResult {
    pub fn success(&self) -> bool {
        self.exit_code == Some(0)
    }
    pub fn stdout_str(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }
    pub fn stderr_str(&self) -> String {
        String::from_utf8_lossy(&self.stderr).into_owned()
    }
}
