//! Container sandbox. Everything the agent produces runs here, never on the host.
//!
//! [`SandboxBackend`] is the abstraction; [`BollardBackend`] implements it for both Podman and
//! Docker, which differ only in the socket path and a few capabilities (overlay mounts).

mod bollard_backend;
pub mod caches;
mod network;
mod types;

pub use bollard_backend::{BollardBackend, detect_socket};
pub use network::EgressPlan;
pub use types::*;

use async_trait::async_trait;
use futures::StreamExt;

pub type Result<T, E = anyhow::Error> = std::result::Result<T, E>;

/// Label set on every container, network and volume the harness creates.
pub const MANAGED_LABEL: &str = "nucleus.managed";

#[async_trait]
pub trait SandboxBackend: Send + Sync {
    fn engine(&self) -> Engine;

    /// Make sure `image` is available locally, pulling it if needed. Returns the image id, which
    /// identifies the exact image content (used for template staleness checks).
    async fn ensure_image(&self, image: &str) -> Result<String>;

    /// Create and start a long-running container, including its network setup.
    async fn create(&self, spec: &ContainerSpec) -> Result<ContainerInfo>;

    /// Start a process inside a running container.
    async fn exec(&self, container: &str, spec: &ExecSpec) -> Result<ExecHandle>;

    /// Stop and remove a container together with any network resources created for it.
    /// Removing a container that does not exist is not an error.
    async fn remove(&self, name: &str) -> Result<()>;

    /// Containers created by the harness, optionally filtered by a label value.
    async fn list(&self, label: Option<(&str, &str)>) -> Result<Vec<ContainerInfo>>;

    /// Create a named volume if it does not exist.
    async fn ensure_volume(&self, name: &str) -> Result<()>;

    /// Run a process to completion and collect its output.
    async fn exec_collect(&self, container: &str, spec: &ExecSpec) -> Result<ExecResult> {
        let mut handle = self.exec(container, spec).await?;
        let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
        while let Some(chunk) = handle.output.next().await {
            match chunk? {
                ExecChunk::Stdout(b) => stdout.extend(b),
                ExecChunk::Stderr(b) => stderr.extend(b),
            }
        }
        let exit_code = handle.wait().await?;
        Ok(ExecResult { exit_code, stdout, stderr })
    }
}
