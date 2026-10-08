use std::path::Path;

use anyhow::{Context, bail};
use nucleus_promotion::{FileChange, Library, NewProposal, Proposal};
use nucleus_sandbox::{ExecSpec, SandboxBackend};

use crate::ToolManifest;

const MAX_FILE: u64 = 512 * 1024;
const MAX_FILES: usize = 64;

#[derive(Debug, Clone)]
pub struct CandidateReport {
    pub proposal: Proposal,
    pub test_log: String,
}

/// Validate a tool candidate, run its test inside `container`, and propose it.
///
/// `host_dir` is the candidate on the host (inside the conversation outbox) and
/// `container_dir` the same directory as seen from inside the container.
pub async fn promote_candidate(
    library: &Library,
    sandbox: &dyn SandboxBackend,
    container: &str,
    host_dir: &Path,
    container_dir: &str,
    rationale: &str,
    source: Option<String>,
) -> crate::Result<CandidateReport> {
    let manifest = ToolManifest::load(host_dir)?;
    let dir_name = host_dir
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    if dir_name != manifest.name {
        bail!(
            "tool directory {dir_name} does not match manifest name {}",
            manifest.name
        );
    }
    let Some(test) = manifest.test.clone() else {
        bail!(
            "tool {} has no test command; add `test = \"...\"` to tool.toml",
            manifest.name
        );
    };
    let mut spec = ExecSpec::new(["sh", "-c", test.as_str()]);
    spec.workdir = Some(container_dir.to_string());
    spec.env.insert("NUCLEUS_WORKSPACE".into(), "/workspace".into());
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(manifest.timeout_secs.max(60) * 2),
        sandbox.exec_collect(container, &spec),
    )
    .await
    .context("tool test timed out")??;
    let test_log = format!("{}{}", result.stdout_str(), result.stderr_str());
    if !result.success() {
        bail!(
            "test for tool {} failed (exit {:?}):\n{test_log}",
            manifest.name,
            result.exit_code
        );
    }

    let mut changes = Vec::new();
    collect(host_dir, host_dir, &manifest.name, &mut changes)?;
    let exists = library.root().join(&manifest.name).exists();
    let proposal = library
        .propose(NewProposal {
            title: format!("{} tool {}", if exists { "Update" } else { "Add" }, manifest.name),
            rationale: format!(
                "{}\n\nTest passed in the sandbox:\n{}",
                rationale.trim(),
                test_log.trim()
            ),
            changes,
            source,
        })
        .await?;
    Ok(CandidateReport { proposal, test_log })
}

fn collect(root: &Path, dir: &Path, name: &str, out: &mut Vec<FileChange>) -> crate::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    for e in std::fs::read_dir(dir)? {
        let e = e?;
        let meta = std::fs::symlink_metadata(e.path())?;
        let rel = e.path().strip_prefix(root)?.to_string_lossy().to_string();
        if meta.file_type().is_symlink() {
            bail!("tool files must not be symlinks: {rel}");
        }
        if meta.is_dir() {
            if matches!(
                e.file_name().to_str(),
                Some("node_modules" | "__pycache__" | ".git" | "target")
            ) {
                continue;
            }
            collect(root, &e.path(), name, out)?;
            continue;
        }
        if meta.len() > MAX_FILE {
            bail!("tool file {rel} is larger than {MAX_FILE} bytes");
        }
        let content =
            String::from_utf8(std::fs::read(e.path())?).with_context(|| format!("tool file {rel} is not text"))?;
        out.push(FileChange {
            path: format!("{name}/{rel}"),
            content: Some(content),
            executable: meta.permissions().mode() & 0o111 != 0,
        });
        if out.len() > MAX_FILES {
            bail!("tool has more than {MAX_FILES} files");
        }
    }
    Ok(())
}
