//! Self-built tools.
//!
//! A tool is `<name>/tool.toml` plus scripts in the tools library. Tools always run inside the
//! sandbox, never on the host, so a bad or malicious tool can at worst damage the container and
//! the mounted worktree, which git can recover.
//!
//! Promotion: the agent writes a tool and a test into its outbox, the harness runs the test in
//! the sandbox, and only a passing tool becomes a proposal the user can approve.
//!
//! The Claude CLI reaches tools through [`MCP_SERVER_JS`], an MCP server that runs inside the
//! container and reads the [`tools_index`] the harness writes.

mod manifest;
mod promote;
mod script;

pub use manifest::{MANIFEST_FILE, ToolManifest};
pub use promote::{CandidateReport, promote_candidate};
pub use script::ScriptTool;

use std::path::Path;

use async_trait::async_trait;
use nucleus_sandbox::SandboxBackend;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub type Result<T, E = anyhow::Error> = std::result::Result<T, E>;

/// The MCP server script mounted into containers.
pub const MCP_SERVER_JS: &str = include_str!("../support/mcp-server.js");

/// Where the approved tools library is mounted inside containers.
pub const TOOLS_MOUNT: &str = "/nucleus/tools";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolOutput {
    pub content: String,
    pub is_error: bool,
}

/// A capability the agent can call.
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn input_schema(&self) -> &Value;
    /// Run the tool inside `container`.
    async fn call(&self, sandbox: &dyn SandboxBackend, container: &str, input: Value) -> Result<ToolOutput>;
}

/// All approved tools in the library at `root`. Invalid manifests are skipped with a warning.
pub fn load_registry(root: &Path) -> Vec<ScriptTool> {
    let mut tools = Vec::new();
    let Ok(entries) = std::fs::read_dir(root) else {
        return tools;
    };
    for e in entries.flatten() {
        if !e.path().join(MANIFEST_FILE).is_file() {
            continue;
        }
        match ToolManifest::load(&e.path()) {
            Ok(m) => tools.push(ScriptTool::new(
                m,
                format!("{TOOLS_MOUNT}/{}", e.file_name().to_string_lossy()),
            )),
            Err(err) => eprintln!("skipping tool {}: {err:#}", e.path().display()),
        }
    }
    tools.sort_by(|a, b| a.name().cmp(b.name()));
    tools
}

/// JSON index of tools for the in-container MCP server.
pub fn tools_index(tools: &[ScriptTool]) -> Value {
    Value::Array(
        tools
            .iter()
            .map(|t| {
                serde_json::json!({
                    "name": t.name(),
                    "description": t.description(),
                    "input_schema": t.input_schema(),
                    "dir": t.container_dir(),
                    "run": t.manifest().run,
                    "timeout_secs": t.manifest().timeout_secs,
                })
            })
            .collect(),
    )
}

/// MCP configuration for the Claude CLI, pointing at the in-container server.
pub fn mcp_config(support_dir_in_container: &str) -> Value {
    serde_json::json!({
        "mcpServers": {
            "nucleus": {
                "command": "node",
                "args": [format!("{support_dir_in_container}/mcp-server.js")],
                "env": { "NUCLEUS_SUPPORT_DIR": support_dir_in_container }
            }
        }
    })
}
