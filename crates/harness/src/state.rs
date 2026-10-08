use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use nucleus_sandbox::NetworkPolicy;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Agent base image, built from `images/agent/Containerfile`.
    pub image: String,
    pub model: Option<String>,
    /// Environment passed to the Claude CLI: `ANTHROPIC_API_KEY` or `CLAUDE_CODE_OAUTH_TOKEN`.
    pub provider_env: BTreeMap<String, String>,
    /// Network policy for new workspaces.
    pub default_network: NetworkPolicy,
    /// `--permission-mode` for the Claude CLI. The container is the security boundary, so the
    /// default lets the CLI use its tools without prompting.
    pub permission_mode: String,
    /// Resource limits for new sandboxes.
    pub limits: ResourceLimits,
}

/// Permission modes the Claude CLI accepts (verified against Claude Code 2.1).
pub const PERMISSION_MODES: &[&str] = &["bypassPermissions", "acceptEdits", "auto", "dontAsk", "manual", "plan"];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ResourceLimits {
    /// Memory limit in MiB; `None` for no limit.
    pub memory_mb: Option<u64>,
    /// CPU limit in cores (fractions allowed); `None` for no limit.
    pub cpus: Option<f64>,
    /// Maximum number of processes; `None` for no limit.
    pub pids: Option<i64>,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            memory_mb: Some(8192),
            cpus: None,
            pids: Some(4096),
        }
    }
}

impl ResourceLimits {
    pub fn to_sandbox(&self) -> nucleus_sandbox::Limits {
        nucleus_sandbox::Limits {
            memory_bytes: self.memory_mb.map(|m| (m as i64).saturating_mul(1024 * 1024)),
            nano_cpus: self.cpus.map(|c| (c * 1e9).round() as i64),
            pids: self.pids,
        }
    }
}

impl Settings {
    /// Reject values the engine or CLI would refuse later.
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.image.trim().is_empty() {
            anyhow::bail!("image must not be empty");
        }
        if !PERMISSION_MODES.contains(&self.permission_mode.as_str()) {
            anyhow::bail!("permission mode must be one of {}", PERMISSION_MODES.join(", "));
        }
        if let Some(m) = self.limits.memory_mb
            && !(256..=1024 * 1024).contains(&m)
        {
            anyhow::bail!("memory limit must be between 256 MiB and 1 TiB");
        }
        if let Some(c) = self.limits.cpus
            && !(c.is_finite() && (0.1..=1024.0).contains(&c))
        {
            anyhow::bail!("CPU limit must be between 0.1 and 1024 cores");
        }
        if let Some(p) = self.limits.pids
            && !(64..=1_000_000).contains(&p)
        {
            anyhow::bail!("process limit must be between 64 and 1000000");
        }
        Ok(())
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            image: "localhost/nucleus-agent:latest".into(),
            model: None,
            provider_env: BTreeMap::new(),
            default_network: NetworkPolicy::None,
            permission_mode: "bypassPermissions".into(),
            limits: ResourceLimits::default(),
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
    /// Skills the last turn used, until the user rates that turn.
    #[serde(default)]
    pub last_turn_skills: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
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
        use std::io::Write;
        let tmp = path.with_extension("tmp");
        let mut opts = std::fs::OpenOptions::new();
        opts.create(true).write(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&tmp)?;
        f.write_all(&serde_json::to_vec_pretty(self)?)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(tmp, path)?;
        Ok(())
    }

    pub fn workspace(&self, id: &str) -> anyhow::Result<&Workspace> {
        self.workspaces
            .iter()
            .find(|w| w.id == id)
            .ok_or_else(|| anyhow::anyhow!("no workspace {id}"))
    }

    pub fn workspace_mut(&mut self, id: &str) -> anyhow::Result<&mut Workspace> {
        self.workspaces
            .iter_mut()
            .find(|w| w.id == id)
            .ok_or_else(|| anyhow::anyhow!("no workspace {id}"))
    }

    pub fn conversation(&self, id: &str) -> anyhow::Result<&Conversation> {
        self.conversations
            .iter()
            .find(|c| c.id == id)
            .ok_or_else(|| anyhow::anyhow!("no conversation {id}"))
    }

    pub fn conversation_mut(&mut self, id: &str) -> anyhow::Result<&mut Conversation> {
        self.conversations
            .iter_mut()
            .find(|c| c.id == id)
            .ok_or_else(|| anyhow::anyhow!("no conversation {id}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation() {
        let ok = Settings::default();
        ok.validate().unwrap();
        let bad = |f: &dyn Fn(&mut Settings)| {
            let mut s = Settings::default();
            f(&mut s);
            s.validate().unwrap_err().to_string()
        };
        assert!(bad(&|s| s.image = " ".into()).contains("image"));
        assert!(bad(&|s| s.permission_mode = "yolo".into()).contains("permission mode"));
        assert!(bad(&|s| s.limits.memory_mb = Some(10)).contains("memory"));
        assert!(bad(&|s| s.limits.cpus = Some(0.0)).contains("CPU"));
        assert!(bad(&|s| s.limits.cpus = Some(f64::NAN)).contains("CPU"));
        assert!(bad(&|s| s.limits.pids = Some(1)).contains("process"));
        let unlimited = Settings {
            limits: ResourceLimits {
                memory_mb: None,
                cpus: None,
                pids: None,
            },
            ..Default::default()
        };
        unlimited.validate().unwrap();
        for m in PERMISSION_MODES {
            Settings {
                permission_mode: m.to_string(),
                ..Default::default()
            }
            .validate()
            .unwrap();
        }
    }

    #[test]
    fn limits_convert() {
        let l = ResourceLimits {
            memory_mb: Some(512),
            cpus: Some(1.5),
            pids: Some(100),
        }
        .to_sandbox();
        assert_eq!(l.memory_bytes, Some(512 * 1024 * 1024));
        assert_eq!(l.nano_cpus, Some(1_500_000_000));
        assert_eq!(l.pids, Some(100));
    }

    #[test]
    fn old_state_files_load_with_defaults() {
        let s: State = serde_json::from_str(
            r#"{"settings":{"image":"x","model":null,"provider_env":{},"default_network":{"mode":"none"}}}"#,
        )
        .unwrap();
        assert_eq!(s.settings.permission_mode, "bypassPermissions");
        assert_eq!(s.settings.limits, ResourceLimits::default());
    }
}
