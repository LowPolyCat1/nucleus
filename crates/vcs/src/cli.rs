//! Thin wrapper around the git CLI, used for operations gix does not cover.

use std::path::Path;

use anyhow::{Context, bail};
use tokio::process::Command;

/// Run git in `dir` and return trimmed stdout. Hooks are disabled because the harness commits
/// in worktrees whose contents were written by an agent.
pub async fn git(dir: &Path, args: &[&str]) -> crate::Result<String> {
    let out = git_output(dir, args).await?;
    if !out.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim_end().to_string())
}

/// Like [`git`] but returns the raw output regardless of exit status.
pub async fn git_output(dir: &Path, args: &[&str]) -> crate::Result<std::process::Output> {
    Command::new("git")
        .current_dir(dir)
        .args(["-c", "core.hooksPath=/dev/null", "-c", "commit.gpgsign=false"])
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .output()
        .await
        .with_context(|| format!("failed to run git {}", args.join(" ")))
}

/// `-c user.name=… -c user.email=…` fallbacks for operations that write commits or reflogs,
/// added only when the repository (or the user's config) has no identity, so the user's own
/// identity is used whenever it exists.
pub async fn identity_args(dir: &Path) -> Vec<String> {
    let mut args = Vec::new();
    for (key, fallback) in [("user.name", "nucleus"), ("user.email", "nucleus@localhost")] {
        let set = git_output(dir, &["config", "--get", key])
            .await
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !set {
            args.push("-c".to_string());
            args.push(format!("{key}={fallback}"));
        }
    }
    args
}

/// Run git with the identity fallbacks from [`identity_args`].
pub async fn git_with_identity(dir: &Path, args: &[&str]) -> crate::Result<String> {
    let mut all = identity_args(dir).await;
    all.extend(args.iter().map(|a| a.to_string()));
    let refs: Vec<&str> = all.iter().map(String::as_str).collect();
    git(dir, &refs).await
}
