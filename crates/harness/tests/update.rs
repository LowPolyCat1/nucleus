#![cfg(unix)] // Uses sh scripts on the host.
mod common;

use common::*;
use nucleus_core::AgentEvent;
use nucleus_harness::*;
use nucleus_sandbox::Engine;
use nucleus_vcs::{MergeOutcome, RebaseOutcome, Vcs};

#[tokio::test]
async fn update_from_base_with_conflicts_resolved_by_the_agent() {
    let f = fixture(Engine::Docker).await;
    let ws = f.harness.add_workspace(&f.repo, None).await.unwrap();
    let conv = f.harness.create_conversation(&ws.id, "main", "t").await.unwrap();
    f.harness.send_message(&conv.id, "edit").await.unwrap(); // writes notes.txt
    commit_file(&f.repo, "notes.txt", "main version\n", "main edits notes").await;

    let out = f.harness.update_from_base(&conv.id).await.unwrap();
    assert_eq!(
        out,
        MergeOutcome::Conflicts {
            paths: vec!["notes.txt".into()]
        }
    );
    assert_eq!(
        f.harness.merge_state(&conv.id).await.unwrap(),
        Some(vec!["notes.txt".to_string()])
    );
    assert!(
        f.harness
            .rebase_conversation(&conv.id)
            .await
            .unwrap_err()
            .to_string()
            .contains("abort")
    );
    assert!(
        f.harness.update_from_base(&conv.id).await.is_err(),
        "one update at a time"
    );

    // A turn that leaves the markers commits nothing.
    f.mode("noop");
    let vcs = nucleus_vcs::GixVcs::open(&f.repo).unwrap();
    let before = vcs.resolve(&conv.branch).await.unwrap();
    f.harness.send_message(&conv.id, "look").await.unwrap();
    assert_eq!(vcs.resolve(&conv.branch).await.unwrap(), before);
    assert!(f.events().iter().any(|e| matches!(e, HarnessEvent::Agent { event: AgentEvent::Error { message }, .. } if message.contains("Conflict markers remain in notes.txt"))));

    // The agent resolves; the harness concludes the merge.
    f.mode("resolve");
    f.harness.resolve_conflicts(&conv.id).await.unwrap();
    let args = f.last_args().join("\n");
    assert!(args.contains("these files have conflicts:\n- notes.txt"), "{args}");
    let tip = vcs.log(&conv.branch, 1).await.unwrap().remove(0);
    assert_eq!(tip.parents.len(), 2, "merge commit");
    assert_eq!(
        std::fs::read_to_string(conv.worktree.join("notes.txt")).unwrap(),
        "resolved\n"
    );
    assert_eq!(f.harness.merge_state(&conv.id).await.unwrap(), None);
    assert_eq!(
        f.harness.update_from_base(&conv.id).await.unwrap(),
        MergeOutcome::UpToDate
    );
    assert!(
        f.harness
            .resolve_conflicts(&conv.id)
            .await
            .unwrap_err()
            .to_string()
            .contains("no update")
    );
    // Now merging into main is conflict free.
    assert!(matches!(
        f.harness.merge_conversation(&conv.id, "main").await.unwrap(),
        MergeOutcome::FastForward { .. }
    ));
}

#[tokio::test]
async fn abort_and_delete_during_an_update() {
    let f = fixture(Engine::Docker).await;
    let ws = f.harness.add_workspace(&f.repo, None).await.unwrap();
    let conv = f.harness.create_conversation(&ws.id, "main", "t").await.unwrap();
    f.harness.send_message(&conv.id, "edit").await.unwrap();
    commit_file(&f.repo, "notes.txt", "main version\n", "main edits notes").await;
    assert!(
        f.harness
            .abort_update(&conv.id)
            .await
            .unwrap_err()
            .to_string()
            .contains("no update")
    );
    f.harness.update_from_base(&conv.id).await.unwrap();
    f.harness.abort_update(&conv.id).await.unwrap();
    assert_eq!(f.harness.merge_state(&conv.id).await.unwrap(), None);
    assert_eq!(
        std::fs::read_to_string(conv.worktree.join("notes.txt")).unwrap(),
        "turn\n"
    );

    // Deleting mid-update aborts it rather than committing markers.
    f.harness.update_from_base(&conv.id).await.unwrap();
    let out = f
        .harness
        .delete_conversation(
            &conv.id,
            DeleteMode::KeepCopy {
                branch: "local/kept".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(out, DeleteOutcome::Deleted);
    let vcs = nucleus_vcs::GixVcs::open(&f.repo).unwrap();
    let kept = vcs.log("local/kept", 5).await.unwrap();
    assert_eq!(kept[0].parents.len(), 1, "no half-done merge commit");
    let diff = vcs.diff("main~1", "local/kept").await.unwrap();
    assert!(!diff.iter().any(|d| d.patch.contains("<<<<<<<")));
}

#[tokio::test]
async fn clean_updates_rebase_and_sandbox_sync() {
    let f = fixture(Engine::Docker).await;
    let ws = f.harness.add_workspace(&f.repo, None).await.unwrap();
    let conv = f.harness.create_conversation(&ws.id, "main", "t").await.unwrap();
    assert_eq!(
        f.harness.update_from_base(&conv.id).await.unwrap(),
        MergeOutcome::UpToDate
    );
    commit_file(&f.repo, "other.txt", "o\n", "main adds other").await;
    // No agent commits yet: fast-forward.
    assert!(matches!(
        f.harness.update_from_base(&conv.id).await.unwrap(),
        MergeOutcome::FastForward { .. }
    ));
    f.harness.send_message(&conv.id, "edit").await.unwrap();
    commit_file(&f.repo, "third.txt", "3\n", "main adds third").await;
    assert!(matches!(
        f.harness.rebase_conversation(&conv.id).await.unwrap(),
        RebaseOutcome::Rebased { .. }
    ));
    assert_eq!(
        f.harness.rebase_conversation(&conv.id).await.unwrap(),
        RebaseOutcome::UpToDate
    );
    // The sandbox sees the rebased history on the next turn.
    f.mode("gitlog");
    f.harness.send_message(&conv.id, "log").await.unwrap();
    let seen = std::fs::read_to_string(f.fake_dir.join("gitlog")).unwrap();
    assert_eq!(
        seen.lines().collect::<Vec<_>>(),
        ["Agent turn: edit", "main adds third", "main adds other", "initial"]
    );
}

#[tokio::test]
async fn remotes_fetch_and_push_through_the_harness() {
    let f = fixture(Engine::Docker).await;
    let remote = f.dir.path().join("remote.git");
    nucleus_vcs::cli::git(f.dir.path(), &["init", "-q", "--bare", remote.to_str().unwrap()])
        .await
        .unwrap();
    nucleus_vcs::cli::git(&f.repo, &["remote", "add", "origin", remote.to_str().unwrap()])
        .await
        .unwrap();
    let ws = f.harness.add_workspace(&f.repo, None).await.unwrap();
    assert_eq!(f.harness.remotes(&ws.id).await.unwrap(), ["origin"]);
    let conv = f.harness.create_conversation(&ws.id, "main", "t").await.unwrap();
    assert!(
        f.harness
            .push(&ws.id, &conv.branch, "origin")
            .await
            .unwrap_err()
            .to_string()
            .contains("not pushed")
    );
    f.harness.push(&ws.id, "main", "origin").await.unwrap();
    f.harness.fetch(&ws.id, None).await.unwrap();
    assert!(
        f.harness
            .branches(&ws.id)
            .await
            .unwrap()
            .iter()
            .any(|b| b.name == "origin/main")
    );
    assert!(f.harness.fetch(&ws.id, Some("nope")).await.is_err());
}
