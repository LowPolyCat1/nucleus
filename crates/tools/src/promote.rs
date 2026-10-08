use anyhow::{Context, bail};
use nucleus_promotion::{FileChange, Library, NewProposal, Proposal};
use nucleus_sandbox::fsutil::Confined;
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
/// `outbox` is the conversation outbox (confined: nothing outside it can be read), `name` the
/// candidate directory under `tools/`, and `container_dir` that directory inside the container.
#[allow(clippy::too_many_arguments)]
pub async fn promote_candidate(
    library: &Library,
    sandbox: &dyn SandboxBackend,
    container: &str,
    outbox: &Confined,
    name: &str,
    container_dir: &str,
    rationale: &str,
    source: Option<String>,
) -> crate::Result<CandidateReport> {
    let dir = outbox
        .sub(&format!("tools/{name}"))
        .with_context(|| format!("opening candidate tools/{name}"))?;
    let manifest = ToolManifest::load_confined(&dir)?;
    if name != manifest.name {
        bail!("tool directory {name} does not match manifest name {}", manifest.name);
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
    collect(&dir, "", &manifest.name, &mut changes)?;
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

fn collect(dir: &Confined, rel: &str, name: &str, out: &mut Vec<FileChange>) -> crate::Result<()> {
    for e in dir.list(rel)? {
        let path = if rel.is_empty() {
            e.name.clone()
        } else {
            format!("{rel}/{}", e.name)
        };
        if e.is_symlink {
            bail!("tool files must not be symlinks: {path}");
        }
        if e.is_dir {
            if matches!(e.name.as_str(), "node_modules" | "__pycache__" | ".git" | "target") {
                continue;
            }
            collect(dir, &path, name, out)?;
            continue;
        }
        if !e.is_file {
            bail!("tool files must be regular files: {path}");
        }
        let content = dir
            .read_text(&path, MAX_FILE)
            .with_context(|| format!("tool file {path}"))?;
        out.push(FileChange {
            path: format!("{name}/{path}"),
            content: Some(content),
            executable: is_executable(dir, &path),
        });
        if out.len() > MAX_FILES {
            bail!("tool has more than {MAX_FILES} files");
        }
    }
    Ok(())
}

fn is_executable(dir: &Confined, path: &str) -> bool {
    dir.is_executable(path)
}
