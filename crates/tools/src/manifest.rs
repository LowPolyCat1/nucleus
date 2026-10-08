use std::path::Path;

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const MANIFEST_FILE: &str = "tool.toml";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolManifest {
    pub name: String,
    pub description: String,
    /// Shell command run in the tool's directory. Input arrives as JSON on stdin and in
    /// `NUCLEUS_TOOL_INPUT`; stdout is the result; a non-zero exit marks an error.
    pub run: String,
    /// Shell command that exits 0 when the tool works. Required for promotion.
    pub test: Option<String>,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    /// JSON schema for the input. Falls back to `schema.json`, then to any object.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_schema: Option<Value>,
}

fn default_timeout() -> u64 {
    120
}

impl ToolManifest {
    /// Load a manifest from the trusted tools library.
    pub fn load(dir: &Path) -> crate::Result<Self> {
        let path = dir.join(MANIFEST_FILE);
        let text = std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
        let schema = dir.join("schema.json");
        let schema = if schema.is_file() {
            Some(std::fs::read_to_string(schema)?)
        } else {
            None
        };
        Self::from_parts(&text, schema.as_deref()).with_context(|| format!("invalid {}", path.display()))
    }

    /// Load a manifest from a directory the sandbox wrote.
    pub fn load_confined(dir: &nucleus_sandbox::fsutil::Confined) -> crate::Result<Self> {
        let text = dir
            .read_text(MANIFEST_FILE, 64 * 1024)
            .with_context(|| format!("reading {MANIFEST_FILE}"))?;
        let schema = if dir.exists("schema.json") {
            Some(dir.read_text("schema.json", 256 * 1024)?)
        } else {
            None
        };
        Self::from_parts(&text, schema.as_deref()).with_context(|| format!("invalid {MANIFEST_FILE}"))
    }

    fn from_parts(manifest: &str, schema: Option<&str>) -> crate::Result<Self> {
        let mut m: Self = toml::from_str(manifest)?;
        if m.input_schema.is_none() {
            m.input_schema = Some(match schema {
                Some(s) => serde_json::from_str(s).context("invalid schema.json")?,
                None => serde_json::json!({ "type": "object" }),
            });
        }
        m.validate()?;
        Ok(m)
    }

    pub fn validate(&self) -> crate::Result<()> {
        let mut chars = self.name.chars();
        let ok = chars
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
            && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
            && self.name.len() <= 48
            && !self.name.starts_with("propose_");
        if !ok {
            bail!(
                "tool name {:?} must be lowercase letters, digits, '-' or '_' and not start with propose_",
                self.name
            );
        }
        if self.description.trim().is_empty() || self.run.trim().is_empty() {
            bail!("tool {} needs a description and a run command", self.name);
        }
        match &self.input_schema {
            Some(Value::Object(o)) if o.get("type").and_then(Value::as_str) == Some("object") => Ok(()),
            _ => bail!(
                "input_schema of {} must be a JSON schema with type = \"object\"",
                self.name
            ),
        }
    }
}
