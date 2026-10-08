//! The whole harness against a real container engine. Run with `NUCLEUS_ENGINE_TESTS=1`.
//!
//! The agent image is `node:22-alpine` and the Claude CLI is replaced by a fake installed
//! through a dependency template, so this needs no API key and exercises real containers,
//! template builds and mounts, the egress proxy network, turns, commits, tool promotion in the
//! sandbox and teardown.

mod common;

use std::sync::Arc;

use common::commit_file;
use nucleus_harness::*;
use nucleus_promotion::{FileChange, LibraryKind, NewProposal};
use nucleus_sandbox::{BollardBackend, NetworkPolicy, SandboxBackend};

const FAKE_TEMPLATE: &str = r#"
name = "fake-claude"
description = "Stands in for the Claude CLI in tests"
mount = { mode = "readonly" }
path_env = { PATH = ["/deps/fake-claude/bin"] }
[build]
command = '''
mkdir -p bin
cat > bin/claude <<'SH'
#!/bin/sh
echo '{"type":"system","subtype":"init","session_id":"real-1","tools":[],"model":"fake"}'
id -u > whoami.txt
echo "$NUCLEUS_OUTBOX_DIR" > outbox-path.txt
ls /deps/fake-claude/bin > deps.txt
# The sandbox must not reach the internet directly.
if wget -q -T 3 -O /dev/null http://example.com 2>/dev/null; then echo open > net.txt; else echo blocked > net.txt; fi
mkdir -p "$NUCLEUS_OUTBOX_DIR/tools/hello" "$NUCLEUS_OUTBOX_DIR/proposals"
printf 'name = "hello"\ndescription = "Says hello"\nrun = "echo hello"\ntest = "echo hello | grep -q hello"\n' > "$NUCLEUS_OUTBOX_DIR/tools/hello/tool.toml"
echo '{"kind":"tool","name":"hello","rationale":"greets"}' > "$NUCLEUS_OUTBOX_DIR/proposals/1.json"
echo '{"type":"result","subtype":"success","is_error":false,"result":"ok","session_id":"real-1"}'
SH
chmod +x bin/claude
'''
"#;

#[tokio::test]
async fn real_engine_end_to_end() {
    if std::env::var("NUCLEUS_ENGINE_TESTS").is_err() {
        eprintln!("skipping: set NUCLEUS_ENGINE_TESTS=1");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    nucleus_vcs::cli::git(&repo, &["init", "-q", "-b", "main"])
        .await
        .unwrap();
    commit_file(&repo, "README.md", "hi\n", "initial").await;

    let backend = Arc::new(
        BollardBackend::connect_default(dir.path().join("support"))
            .await
            .unwrap(),
    );
    let events = Arc::new(std::sync::Mutex::new(Vec::new()));
    let ev = events.clone();
    let h = Harness::open(
        dir.path().join("data"),
        backend.clone(),
        Arc::new(move |e| ev.lock().unwrap().push(e)),
    )
    .await
    .unwrap();
    let mut settings = Settings {
        image: "node:22-alpine".into(),
        ..Default::default()
    };
    settings
        .provider_env
        .insert("ANTHROPIC_API_KEY".into(), "unused".into());
    h.update_settings(settings).await.unwrap();

    let lib = h.library(LibraryKind::Templates);
    let p = lib
        .propose(NewProposal {
            title: "fake".into(),
            rationale: String::new(),
            changes: vec![FileChange {
                path: "fake-claude/template.toml".into(),
                content: Some(FAKE_TEMPLATE.into()),
                executable: false,
            }],
            source: None,
        })
        .await
        .unwrap();
    lib.approve(&p.id).await.unwrap();

    let ws = h.add_workspace(&repo, None).await.unwrap();
    h.configure_workspace(&ws.id, vec!["fake-claude".into()], NetworkPolicy::None)
        .await
        .unwrap();
    let conv = h.create_conversation(&ws.id, "main", "real").await.unwrap();
    let result = async {
        let summary = h.send_message(&conv.id, "go").await?;
        anyhow::ensure!(!summary.is_error, "turn failed: {summary:?}");
        let read = |f: &str| {
            std::fs::read_to_string(conv.worktree.join(f))
                .unwrap_or_default()
                .trim()
                .to_string()
        };
        assert_eq!(read("outbox-path.txt"), "/nucleus/outbox");
        assert_eq!(read("deps.txt"), "claude");
        assert_eq!(read("net.txt"), "blocked", "no direct internet from the sandbox");
        assert_eq!(read("whoami.txt"), current_uid(), "runs as the host user");
        // Committed on the agent branch, not in the main working copy.
        assert_eq!(h.unmerged_commits(&conv.id).await?.len(), 1);
        assert!(!repo.join("whoami.txt").exists());
        // The tool's test ran in the sandbox and produced a proposal.
        let proposals = h.proposals().await?;
        anyhow::ensure!(
            proposals.iter().any(|p| p.title == "Add tool hello"),
            "{:?} {:?}",
            proposals,
            events.lock().unwrap()
        );
        Ok(())
    }
    .await;
    let deleted = h.delete_conversation(&conv.id, DeleteMode::Discard).await;
    result.unwrap();
    assert_eq!(deleted.unwrap(), DeleteOutcome::Deleted);
    assert!(
        backend
            .list(Some(("nucleus.conversation", &conv.id)))
            .await
            .unwrap()
            .is_empty()
    );
    assert!(!conv.worktree.exists());
}

fn current_uid() -> String {
    nucleus_sandbox::current_user().split(':').next().unwrap().to_string()
}
