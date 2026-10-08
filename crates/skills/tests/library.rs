use nucleus_skills::*;

#[tokio::test]
async fn propose_approve_search_and_track() {
    let dir = tempfile::tempdir().unwrap();
    let lib = SkillLibrary::open(dir.path().join("skills"), dir.path().join("usage.json"))
        .await
        .unwrap();
    let content = "---\nname: cargo-tests\ndescription: Run Rust tests with cargo nextest\nwhen_to_use: When verifying Rust changes\n---\n\nRun `cargo nextest run`.\n";
    let p = lib
        .propose_upsert(content, "Worked well in conv 1", Some("c1".into()))
        .await
        .unwrap();
    assert_eq!(p.title, "Add skill cargo-tests");
    assert!(lib.list().unwrap().is_empty(), "not visible before approval");
    lib.library().approve(&p.id).await.unwrap();

    let other = "---\nname: py-lint\ndescription: Lint Python code with ruff\n---\nruff check .\n";
    let p2 = lib.propose_upsert(other, "", None).await.unwrap();
    lib.library().approve(&p2.id).await.unwrap();

    assert_eq!(lib.list().unwrap().len(), 2);
    assert_eq!(lib.load("cargo-tests").unwrap().body, "Run `cargo nextest run`.\n");
    let hits = lib.search("how do I test rust changes", 5).unwrap();
    assert_eq!(hits[0].meta.name, "cargo-tests");
    assert!(
        lib.index_prompt()
            .unwrap()
            .contains("- py-lint: Lint Python code with ruff")
    );

    lib.usage().record_use("py-lint").unwrap();
    for _ in 0..3 {
        lib.usage().record_outcome("py-lint", Outcome::Failure).unwrap();
    }
    let prune = lib.prune_candidates(90).unwrap();
    assert_eq!(prune.len(), 1);
    assert_eq!(prune[0].meta.name, "py-lint");

    // Usage persists.
    let reopened = SkillLibrary::open(dir.path().join("skills"), dir.path().join("usage.json"))
        .await
        .unwrap();
    assert_eq!(reopened.usage().stats("py-lint").failures, 3);

    let upd = lib
        .propose_upsert(&content.replace("nextest run", "test"), "simpler", None)
        .await
        .unwrap();
    assert_eq!(upd.title, "Update skill cargo-tests");
    let del = lib.propose_delete("py-lint", "harmful", None).await.unwrap();
    lib.library().approve(&del.id).await.unwrap();
    assert_eq!(lib.list().unwrap().len(), 1);
    assert!(
        lib.propose_upsert("---\nname: Bad Name\ndescription: x\n---\n", "", None)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn mark_wrong_moves_a_success_to_negative() {
    let dir = tempfile::tempdir().unwrap();
    let lib = SkillLibrary::open(dir.path().join("s"), dir.path().join("u.json"))
        .await
        .unwrap();
    lib.usage().mark_wrong("x").unwrap();
    assert_eq!(
        (lib.usage().stats("x").successes, lib.usage().stats("x").negative),
        (0, 1)
    );
    lib.usage().record_outcome("x", Outcome::Success).unwrap();
    lib.usage().mark_wrong("x").unwrap();
    assert_eq!(
        (lib.usage().stats("x").successes, lib.usage().stats("x").negative),
        (0, 2)
    );
}
