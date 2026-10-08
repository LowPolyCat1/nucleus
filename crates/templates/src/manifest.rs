use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};

pub const MANIFEST_FILE: &str = "template.toml";

/// Where and how a template is mounted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum MountKind {
    /// Read-only bind mount at `/deps/<name>`. Preferred; safe to share between any number of agents.
    Readonly,
    /// Podman overlay at `/deps/<name>`: tools may write, writes are discarded on exit.
    Overlay,
    /// Mounted inside the worktree at `path`, for tools that require it (e.g. `node_modules`).
    /// The path is added to `info/exclude` so it never shows up as a change.
    Worktree { path: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildSpec {
    /// Base image; defaults to the workspace's agent image so native modules match.
    pub image: Option<String>,
    /// Files from the workspace repository the build depends on, e.g. `package.json` and
    /// `pnpm-lock.yaml`. They are provided read-only at `/src` and hashed into the identity.
    #[serde(default)]
    pub lockfiles: Vec<String>,
    /// Shell command that fills the mount path.
    pub command: String,
    /// Working directory for the command; defaults to the mount path.
    pub workdir: Option<String>,
    /// Hosts the build may reach, e.g. `registry.npmjs.org`. `*` allows everything.
    #[serde(default)]
    pub network: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateManifest {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub mount: MountKind,
    /// Plain environment variables. Two templates setting the same one is an error.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// List variables like `PATH`; entries are prepended in template order.
    #[serde(default)]
    pub path_env: BTreeMap<String, Vec<String>>,
    pub build: BuildSpec,
}

pub const WORKSPACE_MOUNT: &str = "/workspace";

impl TemplateManifest {
    pub fn parse(text: &str) -> crate::Result<Self> {
        let m: Self = toml::from_str(text)?;
        m.validate()?;
        Ok(m)
    }

    pub fn load(dir: &Path) -> crate::Result<Self> {
        let path = dir.join(MANIFEST_FILE);
        let text = std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
        Self::parse(&text).with_context(|| format!("invalid {}", path.display()))
    }

    pub fn validate(&self) -> crate::Result<()> {
        let valid_name = !self.name.is_empty()
            && self
                .name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        if !valid_name {
            bail!(
                "template name {:?} must be non-empty and use only letters, digits, '-' and '_'",
                self.name
            );
        }
        if let MountKind::Worktree { path } = &self.mount {
            let p = Path::new(path);
            if path.is_empty()
                || p.is_absolute()
                || !p.components().all(|c| matches!(c, std::path::Component::Normal(_)))
                || path.starts_with(".git")
            {
                bail!("worktree mount path {path:?} must be a plain relative path");
            }
        }
        for key in self.env.keys().chain(self.path_env.keys()) {
            if key.is_empty() || key.contains('=') {
                bail!("invalid environment variable name {key:?}");
            }
        }
        if let Some(k) = self.env.keys().find(|k| self.path_env.contains_key(*k)) {
            bail!("{k} is set both as a plain and a list variable");
        }
        for f in &self.build.lockfiles {
            if Path::new(f).is_absolute() || f.split('/').any(|c| c == "..") {
                bail!("lockfile {f:?} must be relative to the repository");
            }
        }
        Ok(())
    }

    /// Absolute path inside the container.
    pub fn mount_path(&self) -> String {
        match &self.mount {
            MountKind::Readonly | MountKind::Overlay => format!("/deps/{}", self.name),
            MountKind::Worktree { path } => {
                format!("{WORKSPACE_MOUNT}/{}", path.trim_end_matches('/'))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_validates() {
        let m = TemplateManifest::parse(
            r#"
name = "python"
description = "Python venv"
mount = { mode = "readonly" }
env = { VIRTUAL_ENV = "/deps/python/venv" }
path_env = { PATH = ["/deps/python/venv/bin"] }
[build]
lockfiles = ["requirements.txt"]
command = "python3 -m venv venv && venv/bin/pip install -r /src/requirements.txt"
network = ["pypi.org", "files.pythonhosted.org"]
"#,
        )
        .unwrap();
        assert_eq!(m.mount_path(), "/deps/python");
        let n = TemplateManifest::parse(
            "name = \"node\"\nmount = { mode = \"worktree\", path = \"node_modules\" }\n[build]\ncommand = \"true\"\n",
        )
        .unwrap();
        assert_eq!(n.mount_path(), "/workspace/node_modules");
        assert!(
            TemplateManifest::parse("name = \"a b\"\nmount = { mode = \"readonly\" }\n[build]\ncommand = \"x\"\n")
                .is_err()
        );
        assert!(
            TemplateManifest::parse(
                "name = \"a\"\nmount = { mode = \"worktree\", path = \"../x\" }\n[build]\ncommand = \"x\"\n"
            )
            .is_err()
        );
    }
}
