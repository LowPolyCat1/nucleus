#![cfg(unix)] // Uses sh scripts on the host.
mod common;

use common::*;
use nucleus_core::{AgentEvent, TranscriptEntry};
use nucleus_harness::*;
use nucleus_sandbox::{Engine, MountMode, NetworkPolicy};

#[tokio::test]
async fn conversation_lifecycle_and_turns() {
    let f = fixture(Engine::Docker).await;
    let ws = f.harness.add_workspace(&f.repo, None).await.unwrap();
    assert_eq!(ws.name, "repo");
    assert_eq!(ws.network, NetworkPolicy::None, "network is closed by default");
    assert!(f.harness.add_workspace(&f.repo, None).await.is_err(), "same repo twice");

    let conv = f
        .harness
        .create_conversation(&ws.id, "main", "  Fix it  ")
        .await
        .unwrap();
    assert_eq!(conv.title, "Fix it");
    assert_eq!(conv.branch, format!("agent/{}", conv.id));
    assert!(conv.worktree.join("README.md").exists());
    assert!(f.branches().await.contains(&conv.branch));

    // Container wiring.
    let spec = f.backend.spec(&conv.container).unwrap();
    let bind = |t: &str| {
        spec.binds
            .iter()
            .find(|b| b.target == t)
            .unwrap_or_else(|| panic!("no mount {t}"))
            .clone()
    };
    assert_eq!(bind("/workspace").source, conv.worktree);
    assert_eq!(bind("/workspace").mode, MountMode::ReadWrite);
    assert_eq!(bind("/home/agent/.claude/skills").mode, MountMode::ReadOnly);
    assert_eq!(bind("/nucleus/tools").mode, MountMode::ReadOnly);
    assert_eq!(bind("/nucleus/support").mode, MountMode::ReadOnly);
    assert_eq!(spec.network, NetworkPolicy::None);
    assert!(spec.required_hosts.contains(&"api.anthropic.com".to_string()));
    assert_eq!(spec.labels["nucleus.conversation"], conv.id);
    assert!(spec.volumes.iter().any(|v| v.volume == "nucleus-cache-pnpm"));
    assert!(spec.user.is_some(), "never runs as root by default");

    // A turn: events stream, changes are committed, transcript and session persist.
    let summary = f.harness.send_message(&conv.id, "add notes").await.unwrap();
    assert!(!summary.is_error, "{summary:?}");
    assert_eq!(summary.session_id.as_deref(), Some("sess-42"));
    let events = f.events();
    assert!(events.iter().any(|e| matches!(
        e,
        HarnessEvent::Agent {
            event: AgentEvent::TextDelta { .. },
            ..
        }
    )));
    assert!(events.iter().any(|e| matches!(e, HarnessEvent::Committed { .. })));
    assert!(events.iter().any(|e| matches!(
        e,
        HarnessEvent::Status {
            status: ConversationStatus::Running,
            ..
        }
    )));
    let args = f.last_args();
    // The fake backend translates container paths in argv to host paths.
    let i = args
        .iter()
        .position(|a| a == "--mcp-config")
        .expect("mcp config passed");
    assert!(args[i + 1].ends_with("/support/mcp.json"), "{}", args[i + 1]);
    assert!(args.contains(&"--strict-mcp-config".to_string()));
    assert!(
        args.iter().any(|a| a.contains(&conv.branch)),
        "system prompt names the branch"
    );
    assert!(
        !args.contains(&"--resume".to_string()),
        "first turn starts a new session"
    );
    let diff = f.harness.conversation_diff(&conv.id).await.unwrap();
    assert_eq!(diff.len(), 1);
    assert_eq!(diff[0].path, "notes.txt");
    assert_eq!(f.harness.unmerged_commits(&conv.id).await.unwrap().len(), 1);
    // The main working copy is untouched.
    assert!(!f.repo.join("notes.txt").exists());

    // Second turn resumes the session.
    f.harness.send_message(&conv.id, "more").await.unwrap();
    let args = f.last_args();
    let i = args.iter().position(|a| a == "--resume").expect("resumes");
    assert_eq!(args[i + 1], "sess-42");
    let transcript = f.harness.transcript(&conv.id).unwrap();
    assert_eq!(
        transcript
            .iter()
            .filter(|e| matches!(e, TranscriptEntry::User { .. }))
            .count(),
        2
    );
    assert_eq!(
        f.harness.snapshot().await.conversation(&conv.id).unwrap().status,
        ConversationStatus::Idle
    );

    // Skill usage was tracked from the Skill tool calls.
    let stats = f.harness.skills().usage().stats("rust-tests");
    assert_eq!((stats.uses, stats.successes), (2, 2));

    // A no-change turn creates no commit.
    f.mode("noop");
    let before = f.harness.unmerged_commits(&conv.id).await.unwrap().len();
    f.harness.send_message(&conv.id, "nothing").await.unwrap();
    assert_eq!(f.harness.unmerged_commits(&conv.id).await.unwrap().len(), before);

    // Persisted state survives reopening.
    let state = nucleus_harness::State::load(&f.dir.path().join("data/state.json")).unwrap();
    assert_eq!(state.conversations[0].session_id.as_deref(), Some("sess-42"));
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(f.dir.path().join("data/state.json"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o077, 0, "state holds credentials and must be private");
}

#[tokio::test]
async fn turn_errors() {
    let f = fixture(Engine::Docker).await;
    let ws = f.harness.add_workspace(&f.repo, None).await.unwrap();
    let conv = f.harness.create_conversation(&ws.id, "main", "t").await.unwrap();

    f.mode("crash");
    let summary = f.harness.send_message(&conv.id, "x").await.unwrap();
    assert!(summary.is_error);
    assert_eq!(summary.exit_code, Some(2));
    assert_eq!(
        f.harness.snapshot().await.conversation(&conv.id).unwrap().status,
        ConversationStatus::Error
    );
    assert!(
        f.events()
            .iter()
            .any(|e| matches!(e, HarnessEvent::Agent { event: AgentEvent::Stderr { text }, .. } if text == "boom"))
    );

    // Missing credentials.
    let mut s = f.harness.settings().await;
    s.provider_env.remove("ANTHROPIC_API_KEY");
    f.harness.update_settings(s).await.unwrap();
    let err = f.harness.send_message(&conv.id, "x").await.unwrap_err();
    assert!(err.to_string().contains("ANTHROPIC_API_KEY"));

    // Unknown ids.
    assert!(f.harness.send_message("nope", "x").await.is_err());
    assert!(f.harness.create_conversation("nope", "main", "t").await.is_err());
    assert!(
        f.harness
            .create_conversation(&ws.id, "no-such-branch", "t")
            .await
            .is_err()
    );
    assert_eq!(
        f.harness.snapshot().await.conversations.len(),
        1,
        "failed creates leave nothing behind"
    );
}

#[tokio::test]
async fn concurrent_turns_and_cancel() {
    let f = fixture(Engine::Docker).await;
    let ws = f.harness.add_workspace(&f.repo, None).await.unwrap();
    let conv = f.harness.create_conversation(&ws.id, "main", "t").await.unwrap();
    f.mode("slow");
    let h = std::sync::Arc::new(f.harness);
    let h2 = h.clone();
    let id = conv.id.clone();
    let turn = tokio::spawn(async move { h2.send_message(&id, "long").await });
    // Wait until the turn is running.
    for _ in 0..100 {
        if h.snapshot().await.conversation(&conv.id).unwrap().status == ConversationStatus::Running {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let second = h.send_message(&conv.id, "again").await.unwrap_err();
    assert!(second.to_string().contains("already running"));
    assert!(
        h.delete_conversation(&conv.id, DeleteMode::Discard).await.is_err(),
        "cannot delete mid-turn"
    );

    let started = std::time::Instant::now();
    h.cancel(&conv.id).await.unwrap();
    let summary = tokio::time::timeout(std::time::Duration::from_secs(10), turn)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(started.elapsed() < std::time::Duration::from_secs(10));
    assert!(summary.is_error, "a cancelled turn has no result");
    // Cancelling an idle conversation is harmless.
    h.cancel(&conv.id).await.unwrap();
}

#[tokio::test]
async fn container_failure_rolls_back() {
    let f = fixture(Engine::Docker).await;
    let ws = f.harness.add_workspace(&f.repo, None).await.unwrap();
    f.backend.fail_next_create("engine down");
    let err = f.harness.create_conversation(&ws.id, "main", "t").await.unwrap_err();
    assert!(err.to_string().contains("engine down"));
    assert_eq!(f.branches().await, vec!["main"]);
    assert!(f.harness.snapshot().await.conversations.is_empty());
    let worktrees = std::fs::read_dir(f.dir.path().join("data/worktrees"))
        .map(|r| r.count())
        .unwrap_or(0);
    assert_eq!(worktrees, 0);
}

#[tokio::test]
async fn ensure_container_recreates_missing() {
    let f = fixture(Engine::Docker).await;
    let ws = f.harness.add_workspace(&f.repo, None).await.unwrap();
    let conv = f.harness.create_conversation(&ws.id, "main", "t").await.unwrap();
    use nucleus_sandbox::SandboxBackend;
    f.backend.remove(&conv.container).await.unwrap();
    assert!(f.backend.spec(&conv.container).is_none());
    f.harness.send_message(&conv.id, "x").await.unwrap();
    assert!(f.backend.spec(&conv.container).is_some());
}

#[tokio::test]
async fn workspace_rules() {
    let f = fixture(Engine::Docker).await;
    assert!(
        f.harness
            .add_workspace(&f.dir.path().join("not-a-repo"), None)
            .await
            .is_err()
    );
    let ws = f.harness.add_workspace(&f.repo, Some("named".into())).await.unwrap();
    assert_eq!(ws.name, "named");
    let conv = f.harness.create_conversation(&ws.id, "main", "t").await.unwrap();
    assert!(f.harness.remove_workspace(&ws.id).await.is_err(), "has conversations");
    f.harness
        .delete_conversation(&conv.id, DeleteMode::Check)
        .await
        .unwrap();
    f.harness.remove_workspace(&ws.id).await.unwrap();
    assert!(f.harness.snapshot().await.workspaces.is_empty());
    // Adding from a subdirectory resolves to the repository root.
    std::fs::create_dir_all(f.repo.join("sub")).unwrap();
    let ws = f.harness.add_workspace(&f.repo.join("sub"), None).await.unwrap();
    assert_eq!(ws.repo, f.repo);
}
