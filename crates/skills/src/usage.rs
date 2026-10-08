use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    /// The turn using the skill finished without error.
    Success,
    /// The turn using the skill failed.
    Failure,
    /// The user marked the result as wrong.
    Negative,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SkillStats {
    pub uses: u64,
    pub successes: u64,
    pub failures: u64,
    pub negative: u64,
    pub last_used: Option<i64>,
}

impl SkillStats {
    /// Laplace-smoothed share of good outcomes, 0.5 for an unused skill.
    pub fn reliability(&self) -> f64 {
        (self.successes as f64 + 1.0) / ((self.successes + self.failures + self.negative) as f64 + 2.0)
    }
}

/// Per-skill usage and outcome counters, persisted as JSON.
pub struct UsageStore {
    path: PathBuf,
    data: Mutex<BTreeMap<String, SkillStats>>,
}

impl UsageStore {
    pub fn open(path: &Path) -> crate::Result<Self> {
        let data = match std::fs::read_to_string(path) {
            Ok(t) => serde_json::from_str(&t)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(e) => return Err(e.into()),
        };
        Ok(Self {
            path: path.to_path_buf(),
            data: Mutex::new(data),
        })
    }

    pub fn stats(&self, name: &str) -> SkillStats {
        self.data.lock().unwrap().get(name).cloned().unwrap_or_default()
    }

    pub fn record_use(&self, name: &str) -> crate::Result<()> {
        self.update(name, |s| {
            s.uses += 1;
            s.last_used = Some(chrono::Utc::now().timestamp());
        })
    }

    pub fn record_outcome(&self, name: &str, outcome: Outcome) -> crate::Result<()> {
        self.update(name, |s| match outcome {
            Outcome::Success => s.successes += 1,
            Outcome::Failure => s.failures += 1,
            Outcome::Negative => s.negative += 1,
        })
    }

    fn update(&self, name: &str, f: impl FnOnce(&mut SkillStats)) -> crate::Result<()> {
        let mut data = self.data.lock().unwrap();
        f(data.entry(name.to_string()).or_default());
        if let Some(p) = self.path.parent() {
            std::fs::create_dir_all(p)?;
        }
        let tmp = self.path.with_extension("tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(&*data)?)?;
        std::fs::rename(tmp, &self.path)?;
        Ok(())
    }
}
