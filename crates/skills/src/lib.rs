//! Self-learning skills.
//!
//! A skill is `<name>/SKILL.md` in the skills library: markdown with short frontmatter
//! (`name`, `description`, `when_to_use`), compatible with the SKILL.md format the Claude CLI
//! loads natively. Only descriptions go into the agent's context; full skills load on demand.
//!
//! The agent may propose new or updated skills after a task. Every change goes through the
//! promotion workflow and needs user approval. Usage and outcomes are tracked per skill so stale
//! or harmful skills can be pruned; that mitigates the main risk of learning a wrong lesson from
//! one unusual task.

mod skill;
mod usage;

pub use skill::{Skill, SkillMeta, parse_skill, render_skill};
pub use usage::{Outcome, SkillStats, UsageStore};

use std::path::Path;

use anyhow::{anyhow, bail};
use nucleus_promotion::{FileChange, Library, LibraryKind, NewProposal, Proposal};
use serde::{Deserialize, Serialize};

pub type Result<T, E = anyhow::Error> = std::result::Result<T, E>;

pub const SKILL_FILE: &str = "SKILL.md";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkillSummary {
    #[serde(flatten)]
    pub meta: SkillMeta,
    pub stats: SkillStats,
}

pub struct SkillLibrary {
    library: Library,
    usage: UsageStore,
}

impl SkillLibrary {
    /// Open the skills repository at `repo` with usage data kept in `usage_file` (operational
    /// data, deliberately outside the reviewed repository).
    pub async fn open(repo: impl AsRef<Path>, usage_file: impl AsRef<Path>) -> Result<Self> {
        Ok(Self {
            library: Library::open_or_init(repo, LibraryKind::Skills).await?,
            usage: UsageStore::open(usage_file.as_ref())?,
        })
    }

    pub fn library(&self) -> &Library {
        &self.library
    }

    pub fn usage(&self) -> &UsageStore {
        &self.usage
    }

    /// All approved skills, by name.
    pub fn list(&self) -> Result<Vec<SkillSummary>> {
        let mut out = Vec::new();
        let Ok(entries) = std::fs::read_dir(self.library.root()) else {
            return Ok(out);
        };
        for e in entries.flatten() {
            let file = e.path().join(SKILL_FILE);
            if !file.is_file() {
                continue;
            }
            match std::fs::read_to_string(&file)
                .map_err(anyhow::Error::from)
                .and_then(|t| parse_skill(&t))
            {
                Ok(skill) => out.push(SkillSummary {
                    stats: self.usage.stats(&skill.meta.name),
                    meta: skill.meta,
                }),
                Err(e) => eprintln!("skipping invalid skill {}: {e}", file.display()),
            }
        }
        out.sort_by(|a, b| a.meta.name.cmp(&b.meta.name));
        Ok(out)
    }

    pub fn load(&self, name: &str) -> Result<Skill> {
        validate_name(name)?;
        let text = std::fs::read_to_string(self.library.root().join(name).join(SKILL_FILE))
            .map_err(|_| anyhow!("no skill named {name}"))?;
        parse_skill(&text)
    }

    /// Rank skills by word overlap between `query` and their name, description and
    /// `when_to_use`. Skills with recorded failures rank lower.
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<SkillSummary>> {
        let words = tokens(query);
        let mut scored: Vec<(f64, SkillSummary)> = self
            .list()?
            .into_iter()
            .filter_map(|s| {
                let hay = tokens(&format!(
                    "{} {} {}",
                    s.meta.name.replace(['-', '_'], " "),
                    s.meta.description,
                    s.meta.when_to_use.clone().unwrap_or_default()
                ));
                let hits = words.iter().filter(|w| hay.contains(w)).count();
                (hits > 0).then(|| (hits as f64 * s.stats.reliability(), s))
            })
            .collect();
        scored.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.meta.name.cmp(&b.1.meta.name)));
        Ok(scored.into_iter().take(limit).map(|(_, s)| s).collect())
    }

    /// Compact index of skill descriptions for providers without native skill support.
    pub fn index_prompt(&self) -> Result<String> {
        let skills = self.list()?;
        if skills.is_empty() {
            return Ok(String::new());
        }
        let mut s = String::from("Available skills (load the full text only when relevant):\n");
        for k in skills {
            s.push_str(&format!("- {}: {}", k.meta.name, k.meta.description));
            if let Some(w) = &k.meta.when_to_use {
                s.push_str(&format!(" Use when: {w}"));
            }
            s.push('\n');
        }
        Ok(s)
    }

    /// Propose creating or replacing a skill. `content` is the full SKILL.md text.
    pub async fn propose_upsert(&self, content: &str, rationale: &str, source: Option<String>) -> Result<Proposal> {
        let skill = parse_skill(content)?;
        let name = skill.meta.name.clone();
        validate_name(&name)?;
        let exists = self.library.root().join(&name).join(SKILL_FILE).exists();
        self.library
            .propose(NewProposal {
                title: format!("{} skill {name}", if exists { "Update" } else { "Add" }),
                rationale: rationale.to_string(),
                changes: vec![FileChange {
                    path: format!("{name}/{SKILL_FILE}"),
                    content: Some(content.to_string()),
                    executable: false,
                }],
                source,
            })
            .await
    }

    pub async fn propose_delete(&self, name: &str, rationale: &str, source: Option<String>) -> Result<Proposal> {
        validate_name(name)?;
        if !self.library.root().join(name).exists() {
            bail!("no skill named {name}");
        }
        self.library
            .propose(NewProposal {
                title: format!("Remove skill {name}"),
                rationale: rationale.to_string(),
                changes: vec![FileChange {
                    path: name.to_string(),
                    content: None,
                    executable: false,
                }],
                source,
            })
            .await
    }

    /// Skills that look stale or harmful: failing more often than they succeed after a few
    /// uses, or unused for `unused_days`.
    pub fn prune_candidates(&self, unused_days: i64) -> Result<Vec<SkillSummary>> {
        let now = chrono::Utc::now().timestamp();
        Ok(self
            .list()?
            .into_iter()
            .filter(|s| {
                let st = &s.stats;
                let harmful = st.failures + st.negative >= 3 && st.reliability() < 0.5;
                let unused = st.last_used.is_some_and(|t| now - t > unused_days * 86_400);
                harmful || unused
            })
            .collect())
    }
}

pub fn validate_name(name: &str) -> Result<()> {
    let ok = !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !name.starts_with('-');
    if !ok {
        bail!("skill name {name:?} must be lowercase letters, digits and '-', at most 64 characters");
    }
    Ok(())
}

fn tokens(s: &str) -> Vec<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() > 2)
        .map(str::to_string)
        .collect()
}
