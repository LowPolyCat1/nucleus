use std::sync::Arc;

use async_trait::async_trait;
use futures::{FutureExt, StreamExt};
use nucleus_core::{LaunchSpec, LaunchedProcess, ProcessLauncher, ProcessOutput};
use nucleus_sandbox::{ExecChunk, ExecSpec, SandboxBackend};

/// Launches processes inside one conversation's container.
pub struct SandboxLauncher {
    pub backend: Arc<dyn SandboxBackend>,
    pub container: String,
}

#[async_trait]
impl ProcessLauncher for SandboxLauncher {
    async fn launch(&self, spec: LaunchSpec) -> nucleus_core::Result<LaunchedProcess> {
        let exec = ExecSpec {
            cmd: spec.argv,
            env: spec.env,
            workdir: spec.workdir,
            user: None,
            stdin: false,
        };
        let (output, _stdin, exit) = self
            .backend
            .exec(&self.container, &exec)
            .await?
            .into_parts();
        let output = output
            .map(|c| {
                c.map(|c| match c {
                    ExecChunk::Stdout(b) => ProcessOutput::Stdout(b),
                    ExecChunk::Stderr(b) => ProcessOutput::Stderr(b),
                })
            })
            .boxed();
        let exit = exit.boxed();
        Ok(LaunchedProcess { output, exit })
    }
}
