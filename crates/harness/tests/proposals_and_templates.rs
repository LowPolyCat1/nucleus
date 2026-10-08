mod common;

use common::*;
use nucleus_harness::*;
use nucleus_promotion::{FileChange, LibraryKind, NewProposal};
use nucleus_sandbox::{Engine, MountMode, NetworkPolicy};

#[tokio::test]
async fn outbox_becomes_proposals() {
    let f = fixture(Engine::Docker).await;
    let ws = f.harness.add_workspace(&f.repo, None).await.unwrap();
    let conv = f.harness.create_conversation(&ws.id, "main", "t").await.unwrap();
    f.mode("propose");
    f.harness.send_message(&conv.id, "learn").await.unwrap();

    let proposals = f.harness.proposals().await.unwrap();
    let mut kinds: Vec<_> = proposals.iter().map(|p| (p.kind, p.title.clone())).collect();
    kinds.sort_by(|a, b| a.1.cmp(&b.1));
    assert_eq!(
        kinds,
        vec![
            (LibraryKind::Skills, "Add skill rust-tests".to_string()),
            (LibraryKind::Templates, "Add template py".to_string()),
            (LibraryKind::Tools, "Add tool greet".to_string()),
        ]
    );
    assert!(proposals.iter().all(|p| p.source.as_deref() == Some(conv.id.as_str())));
    let failures: Vec<_> = f
        .events()
        .into_iter()
        .filter_map(|e| match e {
            HarnessEvent::ProposalFailed { kind, error, .. } => Some((kind, error)),
            _ => None,
        })
        .collect();
    assert_eq!(failures.len(), 3, "{failures:?}");
    assert!(
        failures
            .iter()
            .any(|(k, e)| *k == LibraryKind::Tools && e.contains("failed"))
    );
    assert!(failures.iter().any(|(_, e)| e.contains("frontmatter")));
    assert!(failures.iter().any(|(_, e)| e.contains("unreadable")));
    // Entries are processed once.
    let outbox = f.dir.path().join("data/conversations").join(&conv.id).join("outbox");
    assert_eq!(std::fs::read_dir(outbox.join("proposals")).unwrap().count(), 0);
    assert_eq!(std::fs::read_dir(outbox.join("processed")).unwrap().count(), 6);
    f.mode("noop");
    f.harness.send_message(&conv.id, "again").await.unwrap();
    assert_eq!(f.harness.proposals().await.unwrap().len(), 3);

    // Nothing is live before approval.
    assert!(f.harness.skills().list().unwrap().is_empty());
    let support = f.dir.path().join("data/conversations").join(&conv.id).join("support");
    assert_eq!(
        std::fs::read_to_string(support.join("tools.json")).unwrap().trim(),
        "[]"
    );

    let tool = proposals.iter().find(|p| p.kind == LibraryKind::Tools).unwrap();
    let detail = f.harness.proposal(LibraryKind::Tools, &tool.id).await.unwrap();
    assert_eq!(detail.diff[0].path, "greet/tool.toml");
    assert!(detail.proposal.rationale.contains("Test passed"));
    let commit = f.harness.approve(LibraryKind::Tools, &tool.id).await.unwrap();
    assert!(
        std::fs::read_to_string(support.join("tools.json"))
            .unwrap()
            .contains("\"greet\""),
        "index refreshed"
    );

    let skill = proposals.iter().find(|p| p.kind == LibraryKind::Skills).unwrap();
    f.harness.approve(LibraryKind::Skills, &skill.id).await.unwrap();
    assert_eq!(f.harness.skills().list().unwrap()[0].meta.name, "rust-tests");

    let tpl = proposals.iter().find(|p| p.kind == LibraryKind::Templates).unwrap();
    f.harness.reject(LibraryKind::Templates, &tpl.id).await.unwrap();
    assert!(f.harness.proposals().await.unwrap().is_empty());
    assert!(
        f.harness.approve(LibraryKind::Templates, &tpl.id).await.is_err(),
        "rejected proposals are gone"
    );

    f.harness.revert(LibraryKind::Tools, &commit).await.unwrap();
    assert_eq!(
        std::fs::read_to_string(support.join("tools.json")).unwrap().trim(),
        "[]"
    );
    assert_eq!(f.harness.history(LibraryKind::Tools, 10).await.unwrap().len(), 3);
}

async fn add_template(h: &Harness, name: &str, toml: &str) {
    let lib = h.library(LibraryKind::Templates);
    let p = lib
        .propose(NewProposal {
            title: name.into(),
            rationale: String::new(),
            changes: vec![FileChange {
                path: format!("{name}/template.toml"),
                content: Some(toml.into()),
                executable: false,
            }],
            source: None,
        })
        .await
        .unwrap();
    lib.approve(&p.id).await.unwrap();
}

#[tokio::test]
async fn templates_are_built_validated_and_mounted() {
    let f = fixture(Engine::Docker).await;
    commit_file(&f.repo, "deps.lock", "v1\n", "lockfile").await;
    add_template(
        &f.harness,
        "tool",
        "name = \"tool\"\nmount = { mode = \"readonly\" }\npath_env = { PATH = [\"/deps/tool/bin\"] }\nenv = { TOOL_HOME = \"/deps/tool\" }\n[build]\nlockfiles = [\"deps.lock\"]\ncommand = \"mkdir -p bin && cp /src/deps.lock bin/version\"\n",
    )
    .await;
    add_template(&f.harness, "nodemods", "name = \"nodemods\"\nmount = { mode = \"worktree\", path = \"node_modules\" }\n[build]\ncommand = \"touch marker\"\n").await;
    add_template(&f.harness, "clash", "name = \"clash\"\nmount = { mode = \"readonly\" }\nenv = { TOOL_HOME = \"/x\" }\n[build]\ncommand = \"true\"\n").await;
    add_template(&f.harness, "cache-clash", "name = \"cache-clash\"\nmount = { mode = \"readonly\" }\nenv = { CARGO_HOME = \"/x\" }\n[build]\ncommand = \"true\"\n").await;
    add_template(
        &f.harness,
        "over",
        "name = \"over\"\nmount = { mode = \"overlay\" }\n[build]\ncommand = \"true\"\n",
    )
    .await;
    add_template(
        &f.harness,
        "failing",
        "name = \"failing\"\nmount = { mode = \"readonly\" }\n[build]\ncommand = \"echo nope >&2; exit 3\"\n",
    )
    .await;
    assert_eq!(f.harness.available_templates().len(), 6);

    let ws = f.harness.add_workspace(&f.repo, None).await.unwrap();
    let err = |r: anyhow::Result<Workspace>| r.unwrap_err().to_string();
    assert!(
        err(f
            .harness
            .configure_workspace(&ws.id, vec!["tool".into(), "clash".into()], NetworkPolicy::None)
            .await)
        .contains("both set TOOL_HOME")
    );
    assert!(
        err(f
            .harness
            .configure_workspace(&ws.id, vec!["cache-clash".into()], NetworkPolicy::None)
            .await)
        .contains("CARGO_HOME")
    );
    assert!(
        err(f
            .harness
            .configure_workspace(&ws.id, vec!["over".into()], NetworkPolicy::None)
            .await)
        .contains("Podman")
    );
    assert!(
        f.harness
            .configure_workspace(&ws.id, vec!["missing".into()], NetworkPolicy::None)
            .await
            .is_err()
    );
    assert!(
        f.harness
            .configure_workspace(&ws.id, vec!["tool".into(), "tool".into()], NetworkPolicy::None)
            .await
            .is_err()
    );

    let allow = NetworkPolicy::Allowlist(vec!["registry.npmjs.org".into()]);
    f.harness
        .configure_workspace(&ws.id, vec!["tool".into(), "nodemods".into()], allow.clone())
        .await
        .unwrap();
    let status = f.harness.template_status(&ws.id).await.unwrap();
    assert!(status.iter().all(|s| !s.fresh));

    let conv = f.harness.create_conversation(&ws.id, "main", "t").await.unwrap();
    let status = f.harness.template_status(&ws.id).await.unwrap();
    assert!(status.iter().all(|s| s.fresh), "{status:?}");
    let spec = f.backend.spec(&conv.container).unwrap();
    assert_eq!(spec.network, allow);
    let tool = spec.binds.iter().find(|b| b.target == "/deps/tool").unwrap();
    assert_eq!(tool.mode, MountMode::ReadOnly);
    assert_eq!(
        std::fs::read_to_string(tool.source.join("bin/version")).unwrap(),
        "v1\n"
    );
    assert!(spec.env["PATH"].starts_with("/deps/tool/bin:"));
    assert_eq!(spec.env["TOOL_HOME"], "/deps/tool");
    let nm = spec
        .binds
        .iter()
        .find(|b| b.target == "/workspace/node_modules")
        .unwrap();
    assert!(nm.source.join("marker").exists());
    let exclude = std::fs::read_to_string(f.repo.join(".git/info/exclude")).unwrap();
    assert!(exclude.contains("/node_modules"));

    // A lockfile change makes the template stale; the next conversation rebuilds it.
    commit_file(&f.repo, "deps.lock", "v2\n", "bump").await;
    let status = f.harness.template_status(&ws.id).await.unwrap();
    assert!(!status.iter().find(|s| s.name == "tool").unwrap().fresh);
    let conv2 = f.harness.create_conversation(&ws.id, "main", "t2").await.unwrap();
    let tool2 = f
        .backend
        .spec(&conv2.container)
        .unwrap()
        .binds
        .into_iter()
        .find(|b| b.target == "/deps/tool")
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(tool2.source.join("bin/version")).unwrap(),
        "v2\n"
    );

    // A failing build blocks conversation creation with the build log.
    f.harness
        .configure_workspace(&ws.id, vec!["failing".into()], NetworkPolicy::None)
        .await
        .unwrap();
    let e = f.harness.create_conversation(&ws.id, "main", "t3").await.unwrap_err();
    assert!(format!("{e:#}").contains("nope"));
}

#[tokio::test]
async fn overlay_templates_work_on_podman() {
    let f = fixture(Engine::Podman).await;
    add_template(
        &f.harness,
        "over",
        "name = \"over\"\nmount = { mode = \"overlay\" }\n[build]\ncommand = \"true\"\n",
    )
    .await;
    let ws = f.harness.add_workspace(&f.repo, None).await.unwrap();
    f.harness
        .configure_workspace(&ws.id, vec!["over".into()], NetworkPolicy::Full)
        .await
        .unwrap();
    let conv = f.harness.create_conversation(&ws.id, "main", "t").await.unwrap();
    let spec = f.backend.spec(&conv.container).unwrap();
    assert_eq!(
        spec.binds.iter().find(|b| b.target == "/deps/over").unwrap().mode,
        MountMode::Overlay
    );
}
