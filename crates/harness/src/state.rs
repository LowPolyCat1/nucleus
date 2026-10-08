use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use nucleus_sandbox::NetworkPolicy;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Agent base image, built from `images/agent/Containerfile`.
    pub image: String,
    pub model: Option<String>,
    /// Environment passed to the Claude CLI: `ANTHROPIC_API_KEY` or `CLAUDE_CODE_OAUTH_TOKEN`.
    pub provider_env: BTreeMap<String, String>,
    /// Network policy for new workspaces.
    pub default_network: NetworkPolicy,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            image: "localhost/nucleus-agent:latest".into(),
            model: None,
            provider_env: BTreeMap::new(),
            default_network: NetworkPolicy::None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Workspace {
    pub id: String,
    pub name: String,
    pub repo: PathBuf,
    /// Template names in declared order. Earlier templates win on `PATH` clashes.
    pub templates: Vec<String>,
    pub network: NetworkPolicy,
    /// Identity of the last successful build per template.
    #[serde(default)]
    pub template_builds: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConversationStatus {
    Idle,
    Running,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Conversation {
    pub id: String,
    pub workspace_id: String,
    pub title: String,
    pub base_branch: String,
    pub branch: String,
    pub worktree: PathBuf,
    pub container: String,
    pub session_id: Option<String>,
    pub created: i64,
    pub status: ConversationStatus,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    pub settings: Settings,
    pub workspaces: Vec<Workspace>,
    pub conversations: Vec<Conversation>,
}

impl State {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(t) => Ok(serde_json::from_str(&t)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.into()),
        }
    }

    /// Atomic write, readable only by the user (it holds provider credentials).
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        use std::os::unix::fs::OpenOptionsExt;
        use std::io::Write;
        let tmp = path.with_extension("tmp");
        let mut f = std::fs::OpenOptions::new().create(true).write(true).truncate(true).mode(0o600).open(&tmp)?;
        f.write_all(&serde_json::to_vec_pretty(self)?)?;
        f.sync_all()?;
        std::fs::rename(tmp, path)?;
        Ok(())
    }

    pub fn workspace(&self, id: &str) -> anyhow::Result<&Workspace> {
        self.workspaces.iter().find(|w| w.id == id).ok_or_else(|| anyhow::anyhow!("no workspace {id}"))
    }

    pub fn workspace_mut(&mut self, id: &str) -> anyhow::Result<&mut Workspace> {
        self.workspaces.iter_mut().find(|w| w.id == id).ok_or_else(|| anyhow::anyhow!("no workspace {id}"))
    }

    pub fn conversation(&self, id: &str) -> anyhow::Result<&Conversation> {
        self.conversations.iter().find(|c| c.id == id).ok_or_else(|| anyhow::anyhow!("no conversation {id}"))
    }

    pub fn conversation_mut(&mut self, id: &str) -> anyhow::Result<&mut Conversation> {
        self.conversations.iter_mut().find(|c| c.id == id).ok_or_else(|| anyhow::anyhow!("no conversation {id}"))
    }
}
