use nucleus_promotion::*;

fn change(path: &str, content: Option<&str>) -> FileChange {
    FileChange {
        path: path.into(),
        content: content.map(str::to_string),
        executable: false,
    }
}

#[tokio::test]
async fn propose_approve_reject_revert() {
    let dir = tempfile::tempdir().unwrap();
    let lib = Library::open_or_init(dir.path().join("skills"), LibraryKind::Skills)
        .await
        .unwrap();

    let p = lib
        .propose(NewProposal {
            title: "Add rust skill".into(),
            rationale: "Learned how to run clippy".into(),
            changes: vec![change("rust/SKILL.md", Some("# Rust\n"))],
            source: Some("conv-1".into()),
        })
        .await
        .unwrap();
    assert_eq!(p.title, "Add rust skill");
    assert_eq!(p.source.as_deref(), Some("conv-1"));
    assert_eq!(p.rationale, "Learned how to run clippy");
    // Not visible before approval.
    assert!(!lib.path("rust/SKILL.md").exists());
    assert_eq!(lib.proposals().await.unwrap().len(), 1);
    assert_eq!(lib.diff(&p.id).await.unwrap()[0].path, "rust/SKILL.md");

    let commit = lib.approve(&p.id).await.unwrap();
    assert_eq!(std::fs::read_to_string(lib.path("rust/SKILL.md")).unwrap(), "# Rust\n");
    assert!(lib.proposals().await.unwrap().is_empty());

    let q = lib
        .propose(NewProposal {
            title: "Bad".into(),
            rationale: "".into(),
            changes: vec![change("x.md", Some("x"))],
            source: None,
        })
        .await
        .unwrap();
    lib.reject(&q.id).await.unwrap();
    assert!(lib.proposals().await.unwrap().is_empty());
    assert!(!lib.path("x.md").exists());

    lib.revert(&commit).await.unwrap();
    assert!(!lib.path("rust/SKILL.md").exists());
    assert_eq!(lib.history(10).await.unwrap().len(), 3);

    for bad in ["../escape.md", "/abs.md", ".git/config", ""] {
        let r = lib
            .propose(NewProposal {
                title: "t".into(),
                rationale: "".into(),
                changes: vec![change(bad, Some("x"))],
                source: None,
            })
            .await;
        assert!(r.is_err(), "{bad}");
    }
    // Reopening an existing library works.
    let again = Library::open_or_init(dir.path().join("skills"), LibraryKind::Skills)
        .await
        .unwrap();
    assert_eq!(again.history(10).await.unwrap().len(), 3);
}
