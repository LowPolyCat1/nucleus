use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, anyhow, bail};
use async_trait::async_trait;
use gix::bstr::ByteSlice;

use crate::cli::{git, git_output, git_with_identity};
use crate::diff::render;
use crate::strategy::BranchStrategy;
use crate::{
    BranchInfo, CommitInfo, FileDiff, FileStatus, MergeOutcome, NamespacedStrategy, Result, Vcs, WorktreeInfo,
};

/// [`Vcs`] implementation backed by gix, with git CLI fallback for worktrees and merges.
#[derive(Clone)]
pub struct GixVcs {
    repo: gix::ThreadSafeRepository,
    workdir: PathBuf,
    strategy: Arc<dyn BranchStrategy>,
}

impl GixVcs {
    /// Open the repository whose main working copy is at (or above) `path`.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_with_strategy(path, Arc::new(NamespacedStrategy))
    }

    pub fn open_with_strategy(path: impl AsRef<Path>, strategy: Arc<dyn BranchStrategy>) -> Result<Self> {
        let repo = gix::discover(path.as_ref())
            .with_context(|| format!("no git repository at {}", path.as_ref().display()))?;
        let workdir = repo
            .workdir()
            .ok_or_else(|| anyhow!("bare repositories are not supported"))?
            .to_path_buf();
        Ok(Self {
            repo: repo.into_sync(),
            workdir,
            strategy,
        })
    }

    /// Initialise a new repository with an initial empty commit on `main`. Used for the skills,
    /// tools and templates libraries.
    pub async fn init(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        std::fs::create_dir_all(path)?;
        git(path, &["init", "-q", "-b", "main"]).await?;
        git(
            path,
            &[
                "-c",
                "user.name=nucleus",
                "-c",
                "user.email=nucleus@localhost",
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                "Initialise library",
            ],
        )
        .await?;
        Self::open(path)
    }

    pub fn strategy(&self) -> &Arc<dyn BranchStrategy> {
        &self.strategy
    }

    async fn blocking<T, F>(&self, f: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(gix::Repository) -> Result<T> + Send + 'static,
    {
        let repo = self.repo.clone();
        tokio::task::spawn_blocking(move || f(repo.to_thread_local())).await?
    }
}

fn resolve_id(repo: &gix::Repository, rev: &str) -> Result<gix::ObjectId> {
    let id = repo
        .rev_parse_single(rev)
        .map_err(|e| anyhow!("cannot resolve {rev}: {e}"))?;
    let commit = id
        .object()
        .map_err(|e| anyhow!("{e}"))?
        .peel_to_commit()
        .map_err(|e| anyhow!("{rev} is not a commit: {e}"))?;
    Ok(commit.id)
}

fn commit_info(repo: &gix::Repository, id: gix::ObjectId) -> Result<CommitInfo> {
    let commit = repo.find_commit(id).map_err(|e| anyhow!("{e}"))?;
    let message = commit.message_raw_sloppy().to_str_lossy().to_string();
    let author = commit.author().map_err(|e| anyhow!("{e}"))?;
    let time = commit.time().map_err(|e| anyhow!("{e}"))?.seconds;
    Ok(CommitInfo {
        id: id.to_string(),
        summary: message.lines().next().unwrap_or_default().to_string(),
        author_name: author.name.to_str_lossy().to_string(),
        author_email: author.email.to_str_lossy().to_string(),
        time,
        parents: commit.parent_ids().map(|p| p.to_string()).collect(),
        message,
    })
}

fn walk(
    repo: &gix::Repository,
    tips: Vec<gix::ObjectId>,
    hidden: Vec<gix::ObjectId>,
    limit: usize,
) -> Result<Vec<CommitInfo>> {
    let walk = repo
        .rev_walk(tips)
        .sorting(gix::revision::walk::Sorting::ByCommitTime(Default::default()))
        .with_hidden(hidden)
        .all()
        .map_err(|e| anyhow!("{e}"))?;
    let mut out = Vec::new();
    for info in walk.take(limit) {
        let info = info.map_err(|e| anyhow!("{e}"))?;
        out.push(commit_info(repo, info.id)?);
    }
    Ok(out)
}

fn read_blob(repo: &gix::Repository, id: gix::ObjectId) -> Result<Vec<u8>> {
    if id.is_null() {
        return Ok(Vec::new());
    }
    Ok(repo.find_object(id).map_err(|e| anyhow!("{e}"))?.detach().data)
}

#[async_trait]
impl Vcs for GixVcs {
    fn workdir(&self) -> &Path {
        &self.workdir
    }

    fn common_dir(&self) -> PathBuf {
        self.repo.to_thread_local().common_dir().to_path_buf()
    }

    async fn branches(&self) -> Result<Vec<BranchInfo>> {
        let strategy = self.strategy.clone();
        self.blocking(move |repo| {
            let head = repo.head_name().ok().flatten().map(|n| n.as_bstr().to_string());
            let platform = repo.references().map_err(|e| anyhow!("{e}"))?;
            let mut out = Vec::new();
            for r in platform.all().map_err(|e| anyhow!("{e}"))? {
                let mut r = r.map_err(|e| anyhow!("{e}"))?;
                let full_ref = r.name().as_bstr().to_string();
                let Some(kind) = strategy.classify(&full_ref) else {
                    continue;
                };
                let Ok(id) = r.peel_to_id() else { continue };
                let name = full_ref
                    .strip_prefix("refs/heads/")
                    .or_else(|| full_ref.strip_prefix("refs/remotes/"))
                    .unwrap_or(&full_ref)
                    .to_string();
                out.push(BranchInfo {
                    is_head: head.as_deref() == Some(full_ref.as_str()),
                    name,
                    full_ref,
                    kind,
                    target: id.to_string(),
                });
            }
            out.sort_by(|a, b| a.name.cmp(&b.name));
            Ok(out)
        })
        .await
    }

    async fn resolve(&self, rev: &str) -> Result<String> {
        let rev = rev.to_string();
        self.blocking(move |repo| Ok(resolve_id(&repo, &rev)?.to_string()))
            .await
    }

    async fn create_branch(&self, name: &str, start: &str) -> Result<()> {
        // Resolve with gix (peeling tags), create with the CLI: writing the reflog needs an
        // identity, which the CLI falls back on when none is configured.
        let id = self.resolve(start).await?;
        let full = format!("refs/heads/{name}");
        if git_output(&self.workdir, &["check-ref-format", &full])
            .await
            .map(|o| !o.status.success())
            .unwrap_or(true)
        {
            bail!("invalid branch name {name:?}");
        }
        let out = git_with_identity(
            &self.workdir,
            &[
                "update-ref",
                "-m",
                &format!("branch: Created from {start}"),
                &full,
                &id,
                "",
            ],
        )
        .await;
        out.map(drop).map_err(|e| anyhow!("cannot create branch {name}: {e:#}"))
    }

    async fn delete_branch(&self, name: &str) -> Result<()> {
        // `git branch -D` also cleans up config sections and the reflog.
        git(&self.workdir, &["branch", "-D", "--", name]).await.map(drop)
    }

    async fn log(&self, rev: &str, limit: usize) -> Result<Vec<CommitInfo>> {
        let rev = rev.to_string();
        self.blocking(move |repo| {
            let id = resolve_id(&repo, &rev)?;
            walk(&repo, vec![id], vec![], limit)
        })
        .await
    }

    async fn graph(&self, tips: &[String], limit: usize) -> Result<Vec<CommitInfo>> {
        let tips = tips.to_vec();
        self.blocking(move |repo| {
            let ids = tips.iter().map(|t| resolve_id(&repo, t)).collect::<Result<Vec<_>>>()?;
            if ids.is_empty() {
                return Ok(Vec::new());
            }
            walk(&repo, ids, vec![], limit)
        })
        .await
    }

    async fn unique_commits(&self, rev: &str, hidden: &[String]) -> Result<Vec<CommitInfo>> {
        let (rev, hidden) = (rev.to_string(), hidden.to_vec());
        self.blocking(move |repo| {
            let id = resolve_id(&repo, &rev)?;
            let hidden = hidden
                .iter()
                .map(|h| resolve_id(&repo, h))
                .collect::<Result<Vec<_>>>()?;
            walk(&repo, vec![id], hidden, usize::MAX)
        })
        .await
    }

    async fn merge_base(&self, a: &str, b: &str) -> Result<Option<String>> {
        let (a, b) = (a.to_string(), b.to_string());
        self.blocking(move |repo| {
            let (a, b) = (resolve_id(&repo, &a)?, resolve_id(&repo, &b)?);
            match repo.merge_base(a, b) {
                Ok(id) => Ok(id.map(|id| id.to_string())),
                Err(_) => Ok(None),
            }
        })
        .await
    }

    async fn diff(&self, from: &str, to: &str) -> Result<Vec<FileDiff>> {
        let (from, to) = (from.to_string(), to.to_string());
        self.blocking(move |repo| {
            let tree_of = |rev: &str| -> Result<gix::Tree<'_>> {
                let id = resolve_id(&repo, rev)?;
                repo.find_commit(id)
                    .map_err(|e| anyhow!("{e}"))?
                    .tree()
                    .map_err(|e| anyhow!("{e}"))
            };
            let (old, new) = (tree_of(&from)?, tree_of(&to)?);
            let changes = repo
                .diff_tree_to_tree(&old, &new, None)
                .map_err(|e| anyhow!("diff failed: {e}"))?;
            use gix::object::tree::diff::ChangeDetached as C;
            let mut out = Vec::new();
            for change in changes {
                let (path, old_path, status, old_id, new_id, mode) = match change {
                    C::Addition {
                        location,
                        id,
                        entry_mode,
                        ..
                    } => (
                        location,
                        None,
                        FileStatus::Added,
                        gix::ObjectId::null(id.kind()),
                        id,
                        entry_mode,
                    ),
                    C::Deletion {
                        location,
                        id,
                        entry_mode,
                        ..
                    } => (
                        location,
                        None,
                        FileStatus::Deleted,
                        id,
                        gix::ObjectId::null(id.kind()),
                        entry_mode,
                    ),
                    C::Modification {
                        location,
                        previous_id,
                        id,
                        entry_mode,
                        ..
                    } => (location, None, FileStatus::Modified, previous_id, id, entry_mode),
                    C::Rewrite {
                        source_location,
                        location,
                        source_id,
                        id,
                        entry_mode,
                        copy,
                        ..
                    } => (
                        location,
                        Some(source_location.to_string()),
                        if copy { FileStatus::Copied } else { FileStatus::Renamed },
                        source_id,
                        id,
                        entry_mode,
                    ),
                };
                if mode.is_tree() {
                    continue;
                }
                let r = render(&read_blob(&repo, old_id)?, &read_blob(&repo, new_id)?);
                out.push(FileDiff {
                    path: path.to_string(),
                    old_path,
                    status,
                    binary: r.binary,
                    additions: r.additions,
                    deletions: r.deletions,
                    patch: r.patch,
                });
            }
            out.sort_by(|a, b| a.path.cmp(&b.path));
            Ok(out)
        })
        .await
    }

    async fn add_worktree(&self, path: &Path, branch: &str) -> Result<()> {
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let p = path.to_str().ok_or_else(|| anyhow!("non utf-8 worktree path"))?;
        git(&self.workdir, &["worktree", "add", "--quiet", p, branch])
            .await
            .map(drop)
    }

    async fn remove_worktree(&self, path: &Path) -> Result<()> {
        let p = path.to_str().ok_or_else(|| anyhow!("non utf-8 worktree path"))?;
        let out = git_output(&self.workdir, &["worktree", "remove", "--force", "--force", p]).await?;
        if !out.status.success() {
            // The directory may already be gone (crash, manual delete); drop the stale metadata.
            if path.exists() {
                tokio::fs::remove_dir_all(path).await.ok();
            }
            git(&self.workdir, &["worktree", "prune"]).await?;
        }
        Ok(())
    }

    async fn worktrees(&self) -> Result<Vec<WorktreeInfo>> {
        let out = git(&self.workdir, &["worktree", "list", "--porcelain"]).await?;
        Ok(parse_worktree_list(&out).into_iter().skip(1).collect())
    }

    async fn commit_all(&self, worktree: &Path, message: &str) -> Result<Option<String>> {
        git(worktree, &["add", "-A"]).await?;
        let staged = git_output(worktree, &["diff", "--cached", "--quiet"]).await?;
        if staged.status.success() {
            return Ok(None);
        }
        git(
            worktree,
            &[
                "-c",
                "user.name=nucleus agent",
                "-c",
                "user.email=agent@nucleus.localhost",
                "commit",
                "-q",
                "--no-verify",
                "-m",
                message,
            ],
        )
        .await?;
        Ok(Some(git(worktree, &["rev-parse", "HEAD"]).await?))
    }

    async fn merge(&self, into: &str, from: &str, message: &str) -> Result<MergeOutcome> {
        let into_id = self.resolve(&format!("refs/heads/{into}")).await?;
        let from_id = self.resolve(from).await?;
        let base = self.merge_base(&into_id, &from_id).await?;
        if base.as_deref() == Some(from_id.as_str()) {
            return Ok(MergeOutcome::UpToDate);
        }
        // If the branch is checked out somewhere, merge in that working copy so it stays in sync.
        let checked_out = std::iter::once(WorktreeInfo {
            path: self.workdir.clone(),
            branch: self
                .repo
                .to_thread_local()
                .head_name()
                .ok()
                .flatten()
                .map(|n| n.shorten().to_string()),
        })
        .chain(self.worktrees().await?)
        .find(|w| w.branch.as_deref() == Some(into));
        if let Some(wt) = checked_out {
            let mut args = crate::cli::identity_args(&wt.path).await;
            args.extend(["merge", "--no-edit", "-m", message, &from_id].map(String::from));
            let refs: Vec<&str> = args.iter().map(String::as_str).collect();
            let out = git_output(&wt.path, &refs).await?;
            if !out.status.success() {
                let conflicts = git(&wt.path, &["diff", "--name-only", "--diff-filter=U"])
                    .await
                    .unwrap_or_default();
                git_output(&wt.path, &["merge", "--abort"]).await.ok();
                if conflicts.is_empty() {
                    bail!("merge failed: {}", String::from_utf8_lossy(&out.stderr).trim());
                }
                return Ok(MergeOutcome::Conflicts {
                    paths: conflicts.lines().map(str::to_string).collect(),
                });
            }
            let head = git(&wt.path, &["rev-parse", "HEAD"]).await?;
            return Ok(if base.as_deref() == Some(into_id.as_str()) {
                MergeOutcome::FastForward { commit: head }
            } else {
                MergeOutcome::Merged { commit: head }
            });
        }
        let full = format!("refs/heads/{into}");
        if base.as_deref() == Some(into_id.as_str()) {
            git_with_identity(&self.workdir, &["update-ref", "-m", message, &full, &from_id, &into_id]).await?;
            return Ok(MergeOutcome::FastForward { commit: from_id });
        }
        // Not checked out anywhere: merge without touching any working copy.
        let out = git_output(
            &self.workdir,
            &["merge-tree", "--write-tree", "--name-only", &into_id, &from_id],
        )
        .await?;
        let stdout = String::from_utf8_lossy(&out.stdout).to_string();
        let mut lines = stdout.lines();
        let tree = lines.next().unwrap_or_default().to_string();
        if out.status.code() == Some(1) {
            let paths = lines.take_while(|l| !l.is_empty()).map(str::to_string).collect();
            return Ok(MergeOutcome::Conflicts { paths });
        }
        if !out.status.success() {
            bail!("merge-tree failed: {}", String::from_utf8_lossy(&out.stderr).trim());
        }
        let commit = git_with_identity(
            &self.workdir,
            &["commit-tree", &tree, "-p", &into_id, "-p", &from_id, "-m", message],
        )
        .await?;
        git_with_identity(&self.workdir, &["update-ref", "-m", message, &full, &commit, &into_id]).await?;
        Ok(MergeOutcome::Merged { commit })
    }
}

fn parse_worktree_list(out: &str) -> Vec<WorktreeInfo> {
    let mut list = Vec::new();
    for block in out.split("\n\n") {
        let mut path = None;
        let mut branch = None;
        for line in block.lines() {
            if let Some(p) = line.strip_prefix("worktree ") {
                path = Some(PathBuf::from(p));
            } else if let Some(b) = line.strip_prefix("branch refs/heads/") {
                branch = Some(b.to_string());
            }
        }
        if let Some(path) = path {
            list.push(WorktreeInfo { path, branch });
        }
    }
    list
}
