use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Which namespace a branch belongs to. See [`crate::strategy`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BranchKind {
    /// Remote-tracking state (`refs/remotes/origin/*`).
    Origin,
    /// The user's own branches.
    Local,
    /// Branches owned by agent conversations.
    Agent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchInfo {
    /// Short name, e.g. `main`, `agent/1234`, `origin/main`.
    pub name: String,
    /// Full ref name, e.g. `refs/heads/main`.
    pub full_ref: String,
    pub kind: BranchKind,
    /// Commit id the branch points to.
    pub target: String,
    /// Whether this is the branch checked out in the main working copy.
    pub is_head: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitInfo {
    pub id: String,
    pub summary: String,
    pub message: String,
    pub author_name: String,
    pub author_email: String,
    /// Seconds since the unix epoch.
    pub time: i64,
    pub parents: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileStatus {
    Added,
    Deleted,
    Modified,
    Renamed,
    Copied,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileDiff {
    pub path: String,
    pub old_path: Option<String>,
    pub status: FileStatus,
    pub binary: bool,
    /// The patch was cut short or skipped because the file is very large.
    #[serde(default)]
    pub truncated: bool,
    pub additions: usize,
    pub deletions: usize,
    /// Unified diff hunks without the `---`/`+++` header. Empty for binary files.
    pub patch: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeInfo {
    pub path: PathBuf,
    /// Short branch name, if a branch is checked out.
    pub branch: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MergeOutcome {
    /// `into` already contained `from`.
    UpToDate,
    FastForward {
        commit: String,
    },
    Merged {
        commit: String,
    },
    /// Nothing was changed; these paths conflict.
    Conflicts {
        paths: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RebaseOutcome {
    UpToDate,
    Rebased {
        commit: String,
    },
    /// Nothing was changed; these paths conflict.
    Conflicts {
        paths: Vec<String>,
    },
}
