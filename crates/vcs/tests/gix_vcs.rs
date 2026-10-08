use std::path::Path;

use nucleus_vcs::{BranchKind, FileStatus, GixVcs, MergeOutcome, Vcs, cli::git};

async fn commit_file(dir: &Path, file: &str, content: &str, msg: &str) {
    std::fs::write(dir.join(file), content).unwrap();
    git(dir, &["add", "-A"]).await.unwrap();
    git(
        dir,
        &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", msg],
    )
    .await
    .unwrap();
}

async fn setup() -> (tempfile::TempDir, GixVcs) {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-q", "-b", "main"]).await.unwrap();
    commit_file(dir.path(), "a.txt", "one\ntwo\n", "initial").await;
    let vcs = GixVcs::open(dir.path()).unwrap();
    (dir, vcs)
}

#[tokio::test]
async fn branches_worktrees_diff_and_merge() {
    let (dir, vcs) = setup().await;
    vcs.create_branch("agent/c1", "main").await.unwrap();
    let branches = vcs.branches().await.unwrap();
    assert_eq!(branches.len(), 2);
    let agent = branches.iter().find(|b| b.name == "agent/c1").unwrap();
    assert_eq!(agent.kind, BranchKind::Agent);
    assert!(branches.iter().any(|b| b.name == "main" && b.is_head));

    let wt = dir.path().parent().unwrap().join(format!("wt-{}", std::process::id()));
    vcs.add_worktree(&wt, "agent/c1").await.unwrap();
    assert_eq!(vcs.worktrees().await.unwrap()[0].branch.as_deref(), Some("agent/c1"));

    std::fs::write(wt.join("a.txt"), "one\nTWO\n").unwrap();
    std::fs::write(wt.join("b.txt"), "new\n").unwrap();
    let commit = vcs.commit_all(&wt, "agent work").await.unwrap();
    assert!(commit.is_some());
    assert!(vcs.commit_all(&wt, "nothing").await.unwrap().is_none());

    let unique = vcs.unique_commits("agent/c1", &["main".into()]).await.unwrap();
    assert_eq!(unique.len(), 1);
    assert_eq!(unique[0].summary, "agent work");

    let diff = vcs.diff("main", "agent/c1").await.unwrap();
    assert_eq!(diff.len(), 2);
    assert_eq!(diff[0].path, "a.txt");
    assert_eq!(diff[0].status, FileStatus::Modified);
    assert_eq!((diff[0].additions, diff[0].deletions), (1, 1));
    assert!(diff[0].patch.contains("+TWO"));
    assert_eq!(diff[1].status, FileStatus::Added);

    // main is checked out in the main working copy: fast-forward there.
    let out = vcs.merge("main", "agent/c1", "merge").await.unwrap();
    assert!(matches!(out, MergeOutcome::FastForward { .. }));
    assert_eq!(std::fs::read_to_string(dir.path().join("b.txt")).unwrap(), "new\n");
    assert!(
        vcs.unique_commits("agent/c1", &["main".into()])
            .await
            .unwrap()
            .is_empty()
    );
    assert!(matches!(
        vcs.merge("main", "agent/c1", "m").await.unwrap(),
        MergeOutcome::UpToDate
    ));

    vcs.remove_worktree(&wt).await.unwrap();
    assert!(!wt.exists());
    vcs.delete_branch("agent/c1").await.unwrap();
    assert_eq!(vcs.branches().await.unwrap().len(), 1);
}

#[tokio::test]
async fn merge_without_checkout_and_conflicts() {
    let (dir, vcs) = setup().await;
    vcs.create_branch("local/feature", "main").await.unwrap();
    vcs.create_branch("agent/x", "main").await.unwrap();
    let wt = tempfile::tempdir().unwrap();
    let wt_path = wt.path().join("x");
    vcs.add_worktree(&wt_path, "agent/x").await.unwrap();
    commit_file(&wt_path, "c.txt", "agent\n", "agent").await;
    commit_file(dir.path(), "d.txt", "main\n", "main moves").await;

    // local/feature is not checked out anywhere: fast-forward by ref update.
    let out = vcs.merge("local/feature", "agent/x", "m").await.unwrap();
    assert!(matches!(out, MergeOutcome::FastForward { .. }));

    // Make a true merge on a not-checked-out branch.
    vcs.create_branch("local/other", "main").await.unwrap();
    let out = vcs.merge("local/other", "agent/x", "merge agent").await.unwrap();
    let MergeOutcome::Merged { commit } = out else {
        panic!("{out:?}")
    };
    assert_eq!(vcs.log(&commit, 10).await.unwrap()[0].parents.len(), 2);

    // Conflict.
    commit_file(&wt_path, "a.txt", "agent side\n", "agent edit").await;
    commit_file(dir.path(), "a.txt", "main side\n", "main edit").await;
    vcs.create_branch("local/c", "main").await.unwrap();
    let out = vcs.merge("local/c", "agent/x", "m").await.unwrap();
    assert_eq!(
        out,
        MergeOutcome::Conflicts {
            paths: vec!["a.txt".into()]
        }
    );
    vcs.remove_worktree(&wt_path).await.unwrap();
}
