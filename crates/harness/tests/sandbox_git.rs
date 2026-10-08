#![cfg(unix)] // Uses sh scripts on the host.
mod common;

use common::*;
use nucleus_core::AgentEvent;
use nucleus_harness::*;
use nucleus_sandbox::Engine;
use nucleus_vcs::Vcs;

#[tokio::test]
async fn agent_commits_are_imported_and_the_sandbox_stays_in_sync() {
    let f = fixture(Engine::Docker).await;
    let ws = f.harness.add_workspace(&f.repo, None).await.unwrap();
    let conv = f.harness.create_conversation(&ws.id, "main", "t").await.unwrap();
    let spec = f.backend.spec(&conv.container).unwrap();
    assert!(spec.binds.iter().any(|b| b.target == "/nucleus/git"));
    assert!(
        spec.binds
            .iter()
            .any(|b| b.target == "/nucleus/main-objects" && b.mode == nucleus_sandbox::MountMode::ReadOnly)
    );
    let gitfile = spec.binds.iter().find(|b| b.target == "/workspace/.git").unwrap();
    assert_eq!(
        std::fs::read_to_string(&gitfile.source).unwrap(),
        "gitdir: /nucleus/git\n"
    );

    // Turn 1: the agent commits one file and leaves another uncommitted.
    f.mode("gitcommit");
    f.harness.send_message(&conv.id, "work").await.unwrap();
    let vcs = nucleus_vcs::GixVcs::open(&f.repo).unwrap();
    let log: Vec<String> = vcs
        .log(&conv.branch, 10)
        .await
        .unwrap()
        .into_iter()
        .map(|c| c.summary)
        .collect();
    assert_eq!(log, ["Agent turn: work", "agent: add committed.txt", "initial"]);
    let commits = f
        .events()
        .iter()
        .filter(|e| matches!(e, HarnessEvent::Committed { .. }))
        .count();
    assert_eq!(commits, 2, "import and harness commit both reported");

    // Turn 2: the sandbox sees the harness commit and a clean tree.
    f.mode("gitlog");
    f.harness.send_message(&conv.id, "look").await.unwrap();
    let seen = std::fs::read_to_string(f.fake_dir.join("gitlog")).unwrap();
    assert_eq!(
        seen.lines().collect::<Vec<_>>(),
        ["Agent turn: work", "agent: add committed.txt", "initial"]
    );
    assert_eq!(std::fs::read_to_string(f.fake_dir.join("gitstatus")).unwrap(), "");

    // Turn 3: rewriting synced history is not imported, but the content still is.
    f.mode("gitamend");
    f.harness.send_message(&conv.id, "amend").await.unwrap();
    assert!(f.events().iter().any(
        |e| matches!(e, HarnessEvent::Agent { event: AgentEvent::Error { message }, .. } if message.contains("rewrote"))
    ));
    let log: Vec<String> = vcs
        .log(&conv.branch, 10)
        .await
        .unwrap()
        .into_iter()
        .map(|c| c.summary)
        .collect();
    assert_eq!(log[0], "Agent turn: amend");
    assert!(!log.iter().any(|s| s == "agent: rewritten"));
    let tip = vcs.resolve(&conv.branch).await.unwrap();
    let files = vcs.diff(&format!("{tip}~1"), &tip).await.unwrap();
    assert_eq!(files[0].path, "committed.txt");
    assert!(files[0].patch.contains("+amended"));
    // No temporary refs are left behind.
    let refs = nucleus_vcs::cli::git(&f.repo, &["for-each-ref", "refs/nucleus"])
        .await
        .unwrap();
    assert_eq!(refs, "");
    // The main working copy never changed.
    assert!(!f.repo.join("committed.txt").exists());
}

#[tokio::test]
async fn sandbox_git_is_set_up_before_the_first_turn_and_after_restarts() {
    let f = fixture(Engine::Docker).await;
    let ws = f.harness.add_workspace(&f.repo, None).await.unwrap();
    let conv = f.harness.create_conversation(&ws.id, "main", "t").await.unwrap();
    f.mode("gitlog");
    f.harness.send_message(&conv.id, "x").await.unwrap();
    assert_eq!(std::fs::read_to_string(f.fake_dir.join("gitlog")).unwrap(), "initial\n");
    f.harness.restart_container(&conv.id).await.unwrap();
    f.harness.send_message(&conv.id, "y").await.unwrap();
    assert_eq!(std::fs::read_to_string(f.fake_dir.join("gitlog")).unwrap(), "initial\n");
    assert_eq!(std::fs::read_to_string(f.fake_dir.join("gitstatus")).unwrap(), "");
}
