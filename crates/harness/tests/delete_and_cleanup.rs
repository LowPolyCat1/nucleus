#![cfg(unix)] // Uses sh scripts on the host.
mod common;

use common::*;
use nucleus_harness::*;
use nucleus_sandbox::Engine;
use nucleus_vcs::{GixVcs, MergeOutcome, Vcs};

#[tokio::test]
async fn delete_modes() {
    let f = fixture(Engine::Docker).await;
    let ws = f.harness.add_workspace(&f.repo, None).await.unwrap();

    // Without changes, Check deletes immediately.
    let empty = f.harness.create_conversation(&ws.id, "main", "empty").await.unwrap();
    assert_eq!(
        f.harness
            .delete_conversation(&empty.id, DeleteMode::Check)
            .await
            .unwrap(),
        DeleteOutcome::Deleted
    );
    assert!(!empty.worktree.exists());
    assert!(f.backend.spec(&empty.container).is_none());

    // With unmerged work, Check refuses and leaves everything in place.
    let c = f.harness.create_conversation(&ws.id, "main", "work").await.unwrap();
    f.harness.send_message(&c.id, "edit").await.unwrap();
    match f.harness.delete_conversation(&c.id, DeleteMode::Check).await.unwrap() {
        DeleteOutcome::NeedsConfirmation { unmerged } => assert_eq!(unmerged.len(), 1),
        other => panic!("{other:?}"),
    }
    assert!(c.worktree.exists());

    // Uncommitted leftovers in the worktree also count as unmerged work.
    let d = f.harness.create_conversation(&ws.id, "main", "dirty").await.unwrap();
    std::fs::write(d.worktree.join("stray.txt"), "x").unwrap();
    assert!(matches!(
        f.harness.delete_conversation(&d.id, DeleteMode::Check).await.unwrap(),
        DeleteOutcome::NeedsConfirmation { .. }
    ));

    // Keep a copy, then delete.
    assert!(
        f.harness
            .delete_conversation(
                &c.id,
                DeleteMode::KeepCopy {
                    branch: "agent/sneaky".into()
                }
            )
            .await
            .is_err()
    );
    assert_eq!(
        f.harness
            .delete_conversation(
                &c.id,
                DeleteMode::KeepCopy {
                    branch: "local/work".into()
                }
            )
            .await
            .unwrap(),
        DeleteOutcome::Deleted
    );
    let branches = f.branches().await;
    assert!(branches.contains(&"local/work".to_string()));
    assert!(!branches.contains(&c.branch));
    assert!(!f.dir.path().join("data/conversations").join(&c.id).exists());

    // Merge into main (checked out in the main working copy), then delete.
    assert_eq!(
        f.harness
            .delete_conversation(&d.id, DeleteMode::MergeInto { branch: "main".into() })
            .await
            .unwrap(),
        DeleteOutcome::Deleted
    );
    assert_eq!(std::fs::read_to_string(f.repo.join("stray.txt")).unwrap(), "x");

    // Discard drops the work.
    let e = f.harness.create_conversation(&ws.id, "main", "discard").await.unwrap();
    f.harness.send_message(&e.id, "edit").await.unwrap();
    assert_eq!(
        f.harness.delete_conversation(&e.id, DeleteMode::Discard).await.unwrap(),
        DeleteOutcome::Deleted
    );
    assert_eq!(f.branches().await, vec!["local/work", "main"]);
    assert!(f.harness.snapshot().await.conversations.is_empty());
    assert!(
        f.harness.delete_conversation(&e.id, DeleteMode::Discard).await.is_err(),
        "already gone"
    );
}

#[tokio::test]
async fn merge_conflicts_keep_the_conversation() {
    let f = fixture(Engine::Docker).await;
    let ws = f.harness.add_workspace(&f.repo, None).await.unwrap();
    let c = f.harness.create_conversation(&ws.id, "main", "t").await.unwrap();
    f.harness.send_message(&c.id, "edit").await.unwrap();
    commit_file(&f.repo, "notes.txt", "main version\n", "conflicting").await;
    assert!(
        f.harness.merge_conversation(&c.id, &c.branch).await.is_err(),
        "agent branches are not merge targets"
    );
    let out = f
        .harness
        .delete_conversation(&c.id, DeleteMode::MergeInto { branch: "main".into() })
        .await
        .unwrap();
    assert_eq!(
        out,
        DeleteOutcome::MergeConflicts {
            paths: vec!["notes.txt".into()]
        }
    );
    assert!(c.worktree.exists());
    assert_eq!(
        std::fs::read_to_string(f.repo.join("notes.txt")).unwrap(),
        "main version\n",
        "aborted merge leaves main clean"
    );

    // Merging into a branch that is not checked out works without touching any working copy.
    let vcs = GixVcs::open(&f.repo).unwrap();
    vcs.create_branch("local/review", "main~1").await.unwrap();
    assert!(matches!(
        f.harness.merge_conversation(&c.id, "local/review").await.unwrap(),
        MergeOutcome::FastForward { .. }
    ));
}

#[tokio::test]
async fn cleanup_removes_orphans_only() {
    let f = fixture(Engine::Docker).await;
    let ws = f.harness.add_workspace(&f.repo, None).await.unwrap();
    let keep = f.harness.create_conversation(&ws.id, "main", "keep").await.unwrap();
    let lost = f.harness.create_conversation(&ws.id, "main", "lost").await.unwrap();

    // Simulate a crash that lost the conversation record but left everything else.
    let state_path = f.dir.path().join("data/state.json");
    let mut state = State::load(&state_path).unwrap();
    state.conversations.retain(|c| c.id != lost.id);
    state.save(&state_path).unwrap();
    // Plus a stray agent branch with no worktree, and a user branch that must survive.
    let vcs = GixVcs::open(&f.repo).unwrap();
    vcs.create_branch("agent/ghost", "main").await.unwrap();
    vcs.create_branch("agent-notes", "main").await.unwrap();
    drop(f.harness);
    let harness = Harness::open(
        f.dir.path().join("data"),
        f.backend.clone(),
        std::sync::Arc::new(|_| {}),
    )
    .await
    .unwrap();

    let report = harness.cleanup_orphans().await.unwrap();
    assert_eq!(report.containers, vec![lost.container.clone()]);
    assert!(report.worktrees.contains(&lost.worktree));
    let mut branches = report.branches.clone();
    branches.sort();
    let mut expected = vec!["agent/ghost".to_string(), lost.branch.clone()];
    expected.sort();
    assert_eq!(branches, expected);
    let left = vcs
        .branches()
        .await
        .unwrap()
        .into_iter()
        .map(|b| b.name)
        .collect::<Vec<_>>();
    assert!(
        left.contains(&keep.branch) && left.contains(&"agent-notes".to_string()) && left.contains(&"main".to_string())
    );
    assert!(keep.worktree.exists());
    assert!(f.backend.spec(&keep.container).is_some());
    // Idempotent.
    assert_eq!(harness.cleanup_orphans().await.unwrap(), CleanupReport::default());
}

#[tokio::test]
async fn running_status_is_reset_on_open() {
    let f = fixture(Engine::Docker).await;
    let ws = f.harness.add_workspace(&f.repo, None).await.unwrap();
    let c = f.harness.create_conversation(&ws.id, "main", "t").await.unwrap();
    let path = f.dir.path().join("data/state.json");
    let mut state = State::load(&path).unwrap();
    state.conversation_mut(&c.id).unwrap().status = ConversationStatus::Running;
    state.save(&path).unwrap();
    let h = Harness::open(
        f.dir.path().join("data"),
        f.backend.clone(),
        std::sync::Arc::new(|_| {}),
    )
    .await
    .unwrap();
    assert_eq!(
        h.snapshot().await.conversation(&c.id).unwrap().status,
        ConversationStatus::Idle
    );
}

#[tokio::test]
async fn cleanup_through_a_symlinked_data_dir_and_past_failures() {
    let f = fixture(Engine::Docker).await;
    // Reopen the harness through a symlink to its data dir (like /var -> /private/var on macOS).
    let link = f.dir.path().join("link");
    std::os::unix::fs::symlink(f.dir.path(), &link).unwrap();
    let h = Harness::open(link.join("data"), f.backend.clone(), std::sync::Arc::new(|_| {}))
        .await
        .unwrap();
    let ws = h.add_workspace(&f.repo, None).await.unwrap();
    let lost = h.create_conversation(&ws.id, "main", "lost").await.unwrap();
    let state_path = link.join("data/state.json");
    let mut state = State::load(&state_path).unwrap();
    state.conversations.clear();
    state.save(&state_path).unwrap();
    // An orphaned agent branch the user has checked out cannot be deleted; cleanup goes on.
    let vcs = GixVcs::open(&f.repo).unwrap();
    vcs.create_branch("agent/stuck", "main").await.unwrap();
    nucleus_vcs::cli::git(&f.repo, &["checkout", "-q", "agent/stuck"])
        .await
        .unwrap();
    let h = Harness::open(link.join("data"), f.backend.clone(), std::sync::Arc::new(|_| {}))
        .await
        .unwrap();

    let report = h.cleanup_orphans().await.unwrap();
    assert_eq!(report.containers, vec![lost.container.clone()]);
    assert!(report.branches.contains(&lost.branch), "{report:?}");
    assert_eq!(report.errors.len(), 1, "{report:?}");
    assert!(report.errors[0].contains("agent/stuck"));
    assert!(!lost.worktree.exists());
    let branches = vcs
        .branches()
        .await
        .unwrap()
        .into_iter()
        .map(|b| b.name)
        .collect::<Vec<_>>();
    assert!(branches.contains(&"agent/stuck".to_string()));
    assert!(!branches.contains(&lost.branch));
}
