use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use nucleus_sandbox::{ExecChunk, ExecSpec, SandboxBackend};
use serde_json::Value;
use tokio::io::AsyncWriteExt;

use crate::{Tool, ToolManifest, ToolOutput};

/// A tool implemented by a script in the library, executed in the sandbox.
#[derive(Debug, Clone)]
pub struct ScriptTool {
    manifest: ToolManifest,
    container_dir: String,
}

impl ScriptTool {
    pub fn new(manifest: ToolManifest, container_dir: impl Into<String>) -> Self {
        Self { manifest, container_dir: container_dir.into() }
    }

    pub fn manifest(&self) -> &ToolManifest {
        &self.manifest
    }

    /// The tool's directory inside the container.
    pub fn container_dir(&self) -> &str {
        &self.container_dir
    }
}

#[async_trait]
impl Tool for ScriptTool {
    fn name(&self) -> &str {
        &self.manifest.name
    }

    fn description(&self) -> &str {
        &self.manifest.description
    }

    fn input_schema(&self) -> &Value {
        self.manifest.input_schema.as_ref().expect("validated")
    }

    async fn call(&self, sandbox: &dyn SandboxBackend, container: &str, input: Value) -> crate::Result<ToolOutput> {
        let input = input.to_string();
        let mut spec = ExecSpec::new(["sh", "-c", self.manifest.run.as_str()]);
        spec.workdir = Some(self.container_dir.clone());
        spec.env.insert("NUCLEUS_TOOL_INPUT".into(), input.clone());
        spec.env.insert("NUCLEUS_WORKSPACE".into(), "/workspace".into());
        spec.stdin = true;
        let mut handle = sandbox.exec(container, &spec).await?;
        if let Some(mut stdin) = handle.stdin.take() {
            stdin.write_all(input.as_bytes()).await?;
            stdin.shutdown().await?;
        }
        let collect = async {
            let (mut out, mut err) = (Vec::new(), Vec::new());
            while let Some(chunk) = handle.output.next().await {
                match chunk? {
                    ExecChunk::Stdout(b) => out.extend(b),
                    ExecChunk::Stderr(b) => err.extend(b),
                }
            }
            Ok::<_, anyhow::Error>((out, err, handle.wait().await?))
        };
        match tokio::time::timeout(Duration::from_secs(self.manifest.timeout_secs), collect).await {
            Ok(result) => {
                let (out, err, code) = result?;
                let mut content = String::from_utf8_lossy(&out).into_owned();
                if !err.is_empty() {
                    content.push_str("\n[stderr]\n");
                    content.push_str(&String::from_utf8_lossy(&err));
                }
                Ok(ToolOutput { content, is_error: code != Some(0) })
            }
            Err(_) => Ok(ToolOutput { content: format!("timed out after {}s", self.manifest.timeout_secs), is_error: true }),
        }
    }
}
