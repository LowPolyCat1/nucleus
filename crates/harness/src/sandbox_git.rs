//! Git inside the sandbox.
//!
//! Each conversation's container gets a private git directory at [`GIT_DIR_MOUNT`] whose object
//! store borrows the main repository's objects (mounted read-only) through git alternates. A
//! small file mounted over `/workspace/.git` points git there, so the agent can use plain git
//! (status, diff, log, commit) on its branch.
//!
//! Security: everything that touches the private git directory runs inside the container. The
//! host never runs git against it and never writes into it; it only imports the agent's
//! commits from a bundle file, opened without following symlinks, and only as a fast-forward
//! of the agent branch.

use std::path::Path;

use anyhow::{Context, bail};
use nucleus_sandbox::ExecSpec;
use nucleus_vcs::cli::{git, git_output};

use crate::{Conversation, Harness, Result};

pub(crate) const GIT_DIR_MOUNT: &str = "/nucleus/git";
pub(crate) const MAIN_OBJECTS_MOUNT: &str = "/nucleus/main-objects";
const BUNDLE: &str = ".nucleus-git.bundle";
const MAX_BUNDLE: u64 = 512 * 1024 * 1024;

/// Exit code of the scripts when the image has no git.
const NO_GIT: i64 = 3;

/// Point the private repository at `$TIP` on `$BRANCH` and reset the index to it, keeping the
/// working tree. Idempotent; creates the repository on first use.
const SYNC: &str = r#"set -e
command -v git >/dev/null 2>&1 || exit 3
export GIT_DIR="$NUCLEUS_GIT_DIR"
# Without GIT_WORK_TREE: `git init` refuses a work tree for a bare repository.
[ -f "$GIT_DIR/HEAD" ] || git init -q --bare "$GIT_DIR"
git config core.bare false
git config core.autocrlf false
git config user.name "nucleus agent"
git config user.email agent@nucleus.localhost
mkdir -p "$GIT_DIR/objects/info"
printf '%s\n' "$NUCLEUS_MAIN_OBJECTS" > "$GIT_DIR/objects/info/alternates"
git update-ref "refs/heads/$BRANCH" "$TIP"
git symbolic-ref HEAD "refs/heads/$BRANCH"
GIT_WORK_TREE=/workspace git reset -q
"#;

/// Bundle the agent's commits on `$BRANCH` that the host does not have (`$TIP`). Prints the new
/// tip, or nothing when there is nothing to import.
const EXPORT: &str = r#"set -e
command -v git >/dev/null 2>&1 || exit 3
export GIT_DIR="$NUCLEUS_GIT_DIR"
[ -f "$GIT_DIR/HEAD" ] || exit 0
head=$(git rev-parse -q --verify "refs/heads/$BRANCH^{commit}") || exit 0
[ "$head" = "$TIP" ] && exit 0
out="$NUCLEUS_OUTBOX_DIR/.nucleus-git.bundle"
rm -f "$out"
git bundle create -q "$out" "refs/heads/$BRANCH" "^$TIP" 2>/dev/null || exit 0
echo "$head"
"#;

/// Outcome of importing the agent's own commits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Import {
    Nothing,
    /// Fast-forwarded the agent branch to this commit.
    Imported(String),
    /// The agent rewrote history; its commits were not imported (the working tree still is).
    Rejected(String),
    /// The sandbox image has no git.
    Unavailable,
}

impl Harness {
    fn git_exec(&self, conv: &Conversation, script: &str, tip: &str) -> ExecSpec {
        let mut spec = ExecSpec::new(["sh", "-c", script]);
        spec.env.insert("BRANCH".into(), conv.branch.clone());
        spec.env.insert("TIP".into(), tip.to_string());
        spec.workdir = Some(crate::WORKSPACE_MOUNT.into());
        spec
    }

    /// Make the sandbox's git match the host's agent branch. Returns false when the image has
    /// no git.
    pub(crate) async fn sync_sandbox_git(&self, conv: &Conversation, tip: &str) -> Result<bool> {
        let res = self
            .backend
            .exec_collect(&conv.container, &self.git_exec(conv, SYNC, tip))
            .await?;
        match res.exit_code {
            Some(0) => Ok(true),
            Some(NO_GIT) => Ok(false),
            code => bail!(
                "setting up git in the sandbox failed (exit {code:?}): {}",
                res.stderr_str().trim()
            ),
        }
    }

    /// Bring commits the agent made inside the sandbox onto the host's agent branch.
    pub(crate) async fn import_sandbox_commits(&self, conv: &Conversation, repo: &Path, tip: &str) -> Result<Import> {
        let res = self
            .backend
            .exec_collect(&conv.container, &self.git_exec(conv, EXPORT, tip))
            .await?;
        match res.exit_code {
            Some(0) => {}
            Some(NO_GIT) => return Ok(Import::Unavailable),
            code => bail!(
                "exporting sandbox commits failed (exit {code:?}): {}",
                res.stderr_str().trim()
            ),
        }
        let head = res.stdout_str().trim().to_string();
        if head.is_empty() {
            return Ok(Import::Nothing);
        }
        if head.len() < 40 || !head.chars().all(|c| c.is_ascii_hexdigit()) {
            bail!("unexpected output from the sandbox: {head:?}");
        }
        let (_, outbox, _) = self.conversation_dirs(&conv.id);
        // Copy the bundle out of the agent-writable outbox before git reads it.
        let outbox = nucleus_sandbox::fsutil::Confined::open(&outbox)?;
        let bytes = outbox.read(BUNDLE, MAX_BUNDLE).context("reading the git bundle")?;
        outbox.remove_file(BUNDLE).ok();
        let tmp = self.paths.conversation(&conv.id).join("import.bundle");
        std::fs::write(&tmp, bytes)?;
        let result = self.fetch_bundle(conv, repo, &tmp, &head, tip).await;
        std::fs::remove_file(&tmp).ok();
        result
    }

    async fn fetch_bundle(
        &self,
        conv: &Conversation,
        repo: &Path,
        bundle: &Path,
        head: &str,
        tip: &str,
    ) -> Result<Import> {
        let bundle = bundle.to_str().context("non utf-8 path")?;
        let incoming = format!("refs/nucleus/incoming/{}", conv.id);
        git(repo, &["bundle", "verify", "-q", bundle])
            .await
            .context("the sandbox produced an invalid bundle")?;
        git(
            repo,
            &[
                "fetch",
                "-q",
                "--no-tags",
                "--no-write-fetch-head",
                bundle,
                &format!("+refs/heads/{}:{incoming}", conv.branch),
            ],
        )
        .await?;
        let fetched = git(repo, &["rev-parse", &incoming]).await;
        git_output(repo, &["update-ref", "-d", &incoming]).await.ok();
        let fetched = fetched?;
        if fetched != head {
            bail!("bundle tip {fetched} does not match the reported {head}");
        }
        let is_ancestor = git_output(repo, &["merge-base", "--is-ancestor", tip, &fetched])
            .await?
            .status
            .success();
        if !is_ancestor {
            return Ok(Import::Rejected(fetched));
        }
        nucleus_vcs::cli::git_with_identity(
            repo,
            &[
                "update-ref",
                "-m",
                "nucleus: import sandbox commits",
                &format!("refs/heads/{}", conv.branch),
                &fetched,
                tip,
            ],
        )
        .await?;
        Ok(Import::Imported(fetched))
    }
}
