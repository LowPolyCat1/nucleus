//! The shared promotion workflow for skills, tools and templates.
//!
//! Each library is its own git repository with `main` checked out, so its current content can be
//! read straight from disk. Every change starts as a proposal: a commit on a `proposal/<id>`
//! branch. Nothing reaches `main` until the user approves it, and every approved change can be
//! reverted.

use std::path::{Path, PathBuf};

use anyhow::{Context, anyhow, bail};
use nucleus_vcs::{CommitInfo, FileDiff, GixVcs, MergeOutcome, Vcs};
use serde::{Deserialize, Serialize};

pub type Result<T, E = anyhow::Error> = std::result::Result<T, E>;

const MAIN: &str = "main";
const PREFIX: &str = "proposal/";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LibraryKind {
    Skills,
    Tools,
    Templates,
}

impl LibraryKind {
    pub fn as_str(self) -> &'static str {
        match self {
            LibraryKind::Skills => "skills",
            LibraryKind::Tools => "tools",
            LibraryKind::Templates => "templates",
        }
    }
}

/// A file to write (or delete, when `content` is `None`), relative to the library root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileChange {
    pub path: String,
    pub content: Option<String>,
    #[serde(default)]
    pub executable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewProposal {
    pub title: String,
    pub rationale: String,
    pub changes: Vec<FileChange>,
    /// Conversation the proposal came from, if any.
    pub source: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Proposal {
    pub id: String,
    pub kind: LibraryKind,
    pub title: String,
    pub rationale: String,
    pub source: Option<String>,
    pub commit: String,
    pub created: i64,
}

pub struct Library {
    kind: LibraryKind,
    vcs: GixVcs,
}

impl Library {
    /// Open the library at `path`, initialising a repository there if needed.
    pub async fn open_or_init(path: impl AsRef<Path>, kind: LibraryKind) -> Result<Self> {
        let path = path.as_ref();
        let vcs = if path.join(".git").exists() {
            GixVcs::open(path)?
        } else {
            GixVcs::init(path).await?
        };
        Ok(Self { kind, vcs })
    }

    pub fn kind(&self) -> LibraryKind {
        self.kind
    }

    /// Root of the checked-out `main` branch.
    pub fn root(&self) -> &Path {
        self.vcs.workdir()
    }

    pub fn vcs(&self) -> &GixVcs {
        &self.vcs
    }

    /// Validate paths and record the change on a new proposal branch.
    pub async fn propose(&self, new: NewProposal) -> Result<Proposal> {
        if new.changes.is_empty() {
            bail!("a proposal needs at least one change");
        }
        for c in &new.changes {
            validate_path(&c.path)?;
        }
        let id = uuid::Uuid::new_v4().simple().to_string()[..12].to_string();
        let branch = format!("{PREFIX}{id}");
        self.vcs.create_branch(&branch, MAIN).await?;
        let scratch = tempfile::tempdir()?;
        let wt = scratch.path().join("wt");
        let result = async {
            self.vcs.add_worktree(&wt, &branch).await?;
            for c in &new.changes {
                let target = wt.join(&c.path);
                match &c.content {
                    Some(content) => {
                        if let Some(parent) = target.parent() {
                            tokio::fs::create_dir_all(parent).await?;
                        }
                        tokio::fs::write(&target, content).await?;
                        #[cfg(unix)]
                        if c.executable {
                            use std::os::unix::fs::PermissionsExt;
                            tokio::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755)).await?;
                        }
                    }
                    None if target.is_dir() => tokio::fs::remove_dir_all(&target).await?,
                    None if target.exists() => tokio::fs::remove_file(&target).await?,
                    None => bail!("cannot delete {}: it does not exist", c.path),
                }
            }
            let message = format_message(&new, &id);
            self.vcs
                .commit_all(&wt, &message)
                .await?
                .ok_or_else(|| anyhow!("the proposal does not change anything"))
        }
        .await;
        self.vcs.remove_worktree(&wt).await.ok();
        match result {
            Ok(_) => self.get(&id).await,
            Err(e) => {
                self.vcs.delete_branch(&branch).await.ok();
                Err(e)
            }
        }
    }

    /// Pending proposals, newest first.
    pub async fn proposals(&self) -> Result<Vec<Proposal>> {
        let mut out = Vec::new();
        for b in self.vcs.branches().await? {
            if let Some(id) = b.name.strip_prefix(PREFIX) {
                out.push(self.get(id).await?);
            }
        }
        out.sort_by_key(|p| std::cmp::Reverse(p.created));
        Ok(out)
    }

    pub async fn get(&self, id: &str) -> Result<Proposal> {
        let commit = self
            .vcs
            .log(&format!("refs/heads/{PREFIX}{id}"), 1)
            .await
            .with_context(|| format!("no pending proposal {id}"))?
            .remove(0);
        Ok(parse_message(self.kind, id, &commit))
    }

    pub async fn diff(&self, id: &str) -> Result<Vec<FileDiff>> {
        self.vcs.diff(MAIN, &format!("refs/heads/{PREFIX}{id}")).await
    }

    /// Apply a proposal to `main`. Returns the resulting commit.
    pub async fn approve(&self, id: &str) -> Result<String> {
        let branch = format!("{PREFIX}{id}");
        let proposal = self.get(id).await?;
        let outcome = self
            .vcs
            .merge(
                MAIN,
                &format!("refs/heads/{branch}"),
                &format!("Approve: {}", proposal.title),
            )
            .await?;
        let commit = match outcome {
            MergeOutcome::FastForward { commit } | MergeOutcome::Merged { commit } => commit,
            MergeOutcome::UpToDate => self.vcs.resolve(MAIN).await?,
            MergeOutcome::Conflicts { paths } => {
                bail!(
                    "proposal conflicts with the current library in {}; reject it and propose again",
                    paths.join(", ")
                )
            }
        };
        self.vcs.delete_branch(&branch).await?;
        Ok(commit)
    }

    pub async fn reject(&self, id: &str) -> Result<()> {
        self.get(id).await?;
        self.vcs.delete_branch(&format!("{PREFIX}{id}")).await
    }

    /// Approved history of the library, newest first.
    pub async fn history(&self, limit: usize) -> Result<Vec<CommitInfo>> {
        self.vcs.log(MAIN, limit).await
    }

    /// Undo an approved change with a revert commit on `main`.
    pub async fn revert(&self, commit: &str) -> Result<String> {
        let root = self.root().to_path_buf();
        let info = self.vcs.log(commit, 1).await?.remove(0);
        let mut args = vec![
            "-c",
            "user.name=nucleus",
            "-c",
            "user.email=nucleus@localhost",
            "revert",
            "--no-edit",
        ];
        if info.parents.len() > 1 {
            args.extend(["-m", "1"]);
        }
        args.push(commit);
        nucleus_vcs::cli::git_with_identity(&root, &args).await?;
        self.vcs.resolve(MAIN).await
    }

    /// Absolute path of a file in the approved library.
    pub fn path(&self, rel: &str) -> PathBuf {
        self.root().join(rel)
    }
}

fn validate_path(path: &str) -> Result<()> {
    let p = Path::new(path);
    let ok = !path.is_empty()
        && p.is_relative()
        && p.components().all(|c| matches!(c, std::path::Component::Normal(_)))
        && !path.starts_with(".git");
    if !ok {
        bail!("invalid library path {path:?}");
    }
    Ok(())
}

fn format_message(p: &NewProposal, id: &str) -> String {
    let mut msg = format!(
        "{}\n\n{}\n\nNucleus-Proposal: {id}\n",
        p.title.trim(),
        p.rationale.trim()
    );
    if let Some(src) = &p.source {
        msg.push_str(&format!("Nucleus-Source: {src}\n"));
    }
    msg
}

fn parse_message(kind: LibraryKind, id: &str, c: &CommitInfo) -> Proposal {
    let mut lines = c.message.lines();
    let title = lines.next().unwrap_or_default().to_string();
    let mut rationale = Vec::new();
    let mut source = None;
    for l in lines {
        if let Some(s) = l.strip_prefix("Nucleus-Source: ") {
            source = Some(s.to_string());
        } else if !l.starts_with("Nucleus-Proposal: ") {
            rationale.push(l);
        }
    }
    Proposal {
        id: id.to_string(),
        kind,
        title,
        rationale: rationale.join("\n").trim().to_string(),
        source,
        commit: c.id.clone(),
        created: c.time,
    }
}
