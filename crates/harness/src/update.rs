//! Keeping agent branches current: remotes, merging the base in (with conflicts the agent can
//! resolve), and rebasing.

use anyhow::{anyhow, bail};
use nucleus_core::TurnSummary;
use nucleus_sandbox::fsutil::Confined;
use nucleus_vcs::{MergeOutcome, RebaseOutcome, Vcs};

use crate::{Harness, HarnessEvent, Result};

/// Largest conflicted file the marker check reads; bigger files count as unresolved.
const MAX_CONFLICT_FILE: u64 = 16 * 1024 * 1024;

impl Harness {
    pub async fn remotes(&self, workspace_id: &str) -> Result<Vec<String>> {
        self.vcs_for(workspace_id).await?.remotes().await
    }

    /// Fetch from one remote, or all of them.
    pub async fn fetch(&self, workspace_id: &str, remote: Option<&str>) -> Result<()> {
        self.vcs_for(workspace_id).await?.fetch(remote).await?;
        tracing::info!(workspace = %workspace_id, ?remote, "fetched");
        Ok(())
    }

    /// Push a local branch. Agent branches are private to their conversation and never pushed.
    pub async fn push(&self, workspace_id: &str, branch: &str, remote: &str) -> Result<String> {
        if self.strategy.conversation_of(branch).is_some() {
            bail!("agent branches are not pushed; merge or keep a copy under local/ first");
        }
        let out = self.vcs_for(workspace_id).await?.push(branch, remote).await?;
        tracing::info!(workspace = %workspace_id, %branch, %remote, "pushed");
        Ok(out)
    }

    /// Merge the conversation's base branch into its agent branch. Conflicts stay in the
    /// worktree until [`Harness::resolve_conflicts`] or [`Harness::abort_update`].
    pub async fn update_from_base(&self, conversation_id: &str) -> Result<MergeOutcome> {
        let lock = self.conversation_lock(conversation_id).await;
        let _guard = lock
            .try_lock()
            .map_err(|_| anyhow!("wait for the running turn to finish"))?;
        let conv = self.state.lock().await.conversation(conversation_id)?.clone();
        let vcs = self.vcs_for(&conv.workspace_id).await?;
        let out = vcs
            .merge_in_worktree(
                &conv.worktree,
                &conv.base_branch,
                &format!("Update from {}", conv.base_branch),
            )
            .await?;
        match &out {
            MergeOutcome::Merged { commit } | MergeOutcome::FastForward { commit } => {
                tracing::info!(conversation = %conv.id, %commit, "updated from base");
                self.emit(HarnessEvent::Committed {
                    conversation_id: conv.id.clone(),
                    commit: commit.clone(),
                });
            }
            MergeOutcome::Conflicts { paths } => {
                tracing::info!(conversation = %conv.id, ?paths, "update from base has conflicts")
            }
            MergeOutcome::UpToDate => {}
        }
        Ok(out)
    }

    /// Rebase the agent branch onto its base. Refused while a merge is in progress.
    pub async fn rebase_conversation(&self, conversation_id: &str) -> Result<RebaseOutcome> {
        let lock = self.conversation_lock(conversation_id).await;
        let _guard = lock
            .try_lock()
            .map_err(|_| anyhow!("wait for the running turn to finish"))?;
        let conv = self.state.lock().await.conversation(conversation_id)?.clone();
        let vcs = self.vcs_for(&conv.workspace_id).await?;
        if vcs.merge_conflicts(&conv.worktree).await?.is_some() {
            bail!("finish or abort the update from {} first", conv.base_branch);
        }
        let out = vcs.rebase(&conv.worktree, &conv.base_branch).await?;
        if let RebaseOutcome::Rebased { commit } = &out {
            tracing::info!(conversation = %conv.id, %commit, "rebased onto base");
            self.emit(HarnessEvent::Committed {
                conversation_id: conv.id.clone(),
                commit: commit.clone(),
            });
        }
        Ok(out)
    }

    /// Conflicted paths while an update from the base is in progress.
    pub async fn merge_state(&self, conversation_id: &str) -> Result<Option<Vec<String>>> {
        let conv = self.state.lock().await.conversation(conversation_id)?.clone();
        self.vcs_for(&conv.workspace_id)
            .await?
            .merge_conflicts(&conv.worktree)
            .await
    }

    pub async fn abort_update(&self, conversation_id: &str) -> Result<()> {
        let lock = self.conversation_lock(conversation_id).await;
        let _guard = lock
            .try_lock()
            .map_err(|_| anyhow!("wait for the running turn to finish"))?;
        let conv = self.state.lock().await.conversation(conversation_id)?.clone();
        let vcs = self.vcs_for(&conv.workspace_id).await?;
        if vcs.merge_conflicts(&conv.worktree).await?.is_none() {
            bail!("no update in progress");
        }
        vcs.abort_merge(&conv.worktree).await
    }

    /// Ask the agent to resolve the conflicts of an update in progress. The merge is concluded
    /// by the usual commit after the turn, once no conflict markers remain.
    pub async fn resolve_conflicts(&self, conversation_id: &str) -> Result<TurnSummary> {
        let (paths, base) = {
            let conv = self.state.lock().await.conversation(conversation_id)?.clone();
            let paths = self
                .vcs_for(&conv.workspace_id)
                .await?
                .merge_conflicts(&conv.worktree)
                .await?
                .ok_or_else(|| anyhow!("no update in progress"))?;
            (paths, conv.base_branch)
        };
        let prompt = format!(
            "{base} was merged into your branch and these files have conflicts:\n{}\n\n\
             Edit each file so it keeps the intent of both sides, and remove every conflict marker \
             (<<<<<<<, =======, >>>>>>>). Run the tests if there are any. Do not commit; the harness \
             concludes the merge when no markers remain.",
            paths.iter().map(|p| format!("- {p}")).collect::<Vec<_>>().join("\n")
        );
        self.send_message(conversation_id, &prompt).await
    }

    /// Files among `paths` (relative to the worktree) that still contain conflict markers or
    /// cannot be checked. The worktree is agent-writable, so it is read confined.
    pub(crate) fn unresolved(&self, worktree: &std::path::Path, paths: &[String]) -> Vec<String> {
        let Ok(dir) = Confined::open(worktree) else {
            return paths.to_vec();
        };
        paths
            .iter()
            .filter(|p| match dir.read(p, MAX_CONFLICT_FILE) {
                Ok(bytes) => has_conflict_markers(&bytes),
                // Deleted files resolve a modify/delete conflict; anything else is suspect.
                Err(e) => e.kind() != std::io::ErrorKind::NotFound,
            })
            .cloned()
            .collect()
    }
}

/// Whether a file still has git conflict markers at the start of a line.
pub(crate) fn has_conflict_markers(bytes: &[u8]) -> bool {
    bytes.split(|&b| b == b'\n').any(|line| {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        line.starts_with(b"<<<<<<< ")
            || line.starts_with(b">>>>>>> ")
            || line == b"======="
            || line == b"<<<<<<<"
            || line == b">>>>>>>"
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markers() {
        assert!(has_conflict_markers(b"a\n<<<<<<< HEAD\nx\n=======\ny\n>>>>>>> main\n"));
        assert!(has_conflict_markers(b"=======\r\n"));
        assert!(!has_conflict_markers(
            b"a\n  <<<<<<< not at start\n========= heading underline\n"
        ));
        assert!(!has_conflict_markers(b""));
    }
}
