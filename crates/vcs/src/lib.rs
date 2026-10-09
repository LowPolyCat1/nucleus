//! Version control layer.
//!
//! [`Vcs`] is the generic interface the rest of the harness talks to. [`GixVcs`] implements it with
//! gix for reading refs, commits and trees, and falls back to the git CLI for operations gix does
//! not cover yet (worktrees, merges, committing a dirty worktree).
//!
//! The branching strategy (which namespaces exist and how agent branches are named) lives in
//! [`strategy`] and is deliberately separate from the generic layer, so it can be swapped.

pub mod cli;
mod diff;
pub mod exclude;
mod gix_vcs;
pub mod strategy;
mod types;

pub use gix_vcs::GixVcs;
pub use strategy::{BranchStrategy, NamespacedStrategy};
pub use types::*;

use std::path::{Path, PathBuf};

use async_trait::async_trait;

pub type Result<T, E = anyhow::Error> = std::result::Result<T, E>;

/// Generic version control operations. Branch names are short names (`main`, `agent/123`,
/// `origin/main`); revisions accept anything `git rev-parse` would.
#[async_trait]
pub trait Vcs: Send + Sync {
    /// Root of the main working copy.
    fn workdir(&self) -> &Path;

    /// All local and remote-tracking branches.
    async fn branches(&self) -> Result<Vec<BranchInfo>>;

    /// Resolve a revision to a full commit id.
    async fn resolve(&self, rev: &str) -> Result<String>;

    /// Create a local branch `name` at `start`. Fails if it already exists.
    async fn create_branch(&self, name: &str, start: &str) -> Result<()>;

    /// Delete a local branch. Does not check whether its commits are reachable elsewhere.
    async fn delete_branch(&self, name: &str) -> Result<()>;

    /// Commits reachable from `rev`, newest first.
    async fn log(&self, rev: &str, limit: usize) -> Result<Vec<CommitInfo>>;

    /// Commits reachable from any of `tips`, newest first. Used for the branch tree.
    async fn graph(&self, tips: &[String], limit: usize) -> Result<Vec<CommitInfo>>;

    /// Commits reachable from `rev` but not from any of `hidden`.
    async fn unique_commits(&self, rev: &str, hidden: &[String]) -> Result<Vec<CommitInfo>>;

    /// Best common ancestor of two revisions.
    async fn merge_base(&self, a: &str, b: &str) -> Result<Option<String>>;

    /// File level diff between the trees of two revisions, including unified hunks.
    async fn diff(&self, from: &str, to: &str) -> Result<Vec<FileDiff>>;

    /// Add a worktree at `path` with `branch` checked out.
    async fn add_worktree(&self, path: &Path, branch: &str) -> Result<()>;

    /// Remove a worktree, discarding any uncommitted changes in it.
    async fn remove_worktree(&self, path: &Path) -> Result<()>;

    /// All worktrees except the main one.
    async fn worktrees(&self) -> Result<Vec<WorktreeInfo>>;

    /// Stage everything in `worktree` and commit. Returns `None` if there was nothing to commit.
    async fn commit_all(&self, worktree: &Path, message: &str) -> Result<Option<String>>;

    /// Merge `from` into local branch `into`.
    async fn merge(&self, into: &str, from: &str, message: &str) -> Result<MergeOutcome>;

    /// Path of the git directory shared by all worktrees.
    fn common_dir(&self) -> PathBuf;

    /// Configured remotes.
    async fn remotes(&self) -> Result<Vec<String>>;

    /// Fetch from `remote`, or from every remote when `None`. Prunes deleted branches.
    async fn fetch(&self, remote: Option<&str>) -> Result<()>;

    /// Push local `branch` to the same name on `remote` and set it as upstream.
    async fn push(&self, branch: &str, remote: &str) -> Result<String>;

    /// Rebase the branch checked out in `worktree` onto `onto`. Conflicts abort the rebase and
    /// leave the branch as it was.
    async fn rebase(&self, worktree: &Path, onto: &str) -> Result<RebaseOutcome>;

    /// Merge `from` into the branch checked out in `worktree`. Unlike [`Vcs::merge`],
    /// conflicts are left in the working tree for someone to resolve; see
    /// [`Vcs::merge_conflicts`] and [`Vcs::abort_merge`].
    async fn merge_in_worktree(&self, worktree: &Path, from: &str, message: &str) -> Result<MergeOutcome>;

    /// Paths with unresolved conflicts when a merge is in progress in `worktree`.
    async fn merge_conflicts(&self, worktree: &Path) -> Result<Option<Vec<String>>>;

    /// Abort a merge in progress in `worktree`.
    async fn abort_merge(&self, worktree: &Path) -> Result<()>;
}
