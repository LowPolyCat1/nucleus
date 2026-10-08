//! Builds a template end to end in a real container. Run with `NUCLEUS_ENGINE_TESTS=1`.

use std::collections::BTreeMap;

use nucleus_sandbox::{BollardBackend, ContainerSpec, ExecSpec, SandboxBackend};
use nucleus_templates::*;

#[tokio::test]
async fn build_mount_and_reuse() {
    if std::env::var("NUCLEUS_ENGINE_TESTS").is_err() {
        return;
    }
    let backend = BollardBackend::connect_default(std::env::temp_dir().join("nucleus-tpl-test"))
        .await
        .unwrap();
    let repo = tempfile::tempdir().unwrap();
    std::fs::write(repo.path().join("deps.lock"), "hello-tool 1.0\n").unwrap();
    let manifest = TemplateManifest::parse(
        r#"
name = "hello"
mount = { mode = "readonly" }
env = { HELLO_HOME = "/deps/hello" }
path_env = { PATH = ["/deps/hello/bin"] }
[build]
lockfiles = ["deps.lock"]
command = "echo building; echo warn >&2; printf partial; mkdir -p bin && printf '#!/bin/sh\necho hello from $(cat /deps/hello/version)\n' > bin/hello-tool && chmod +x bin/hello-tool && cp /src/deps.lock version"
"#,
    )
    .unwrap();
    let store = tempfile::tempdir().unwrap();
    let builder = TemplateBuilder {
        backend: &backend,
        root: store.path().into(),
    };
    let lines = std::sync::Mutex::new(Vec::new());
    let first = builder
        .build_streaming(&manifest, repo.path(), "node:22-alpine", &|l| {
            lines.lock().unwrap().push(l.to_string())
        })
        .await
        .unwrap();
    assert!(first.built, "{}", first.log);
    assert_eq!(builder.last_log("hello").unwrap(), first.log);
    let again = builder.build(&manifest, repo.path(), "node:22-alpine").await.unwrap();
    assert!(!again.built);
    assert_eq!(first.identity, again.identity);

    // Mount it into an agent container and use it from PATH.
    let resolved = resolve(
        &[TemplateMount {
            manifest: manifest.clone(),
            built: first.path.clone(),
        }],
        &BTreeMap::new(),
        &BTreeMap::new(),
        false,
    )
    .unwrap();
    let name = format!("nucleus-tpl-use-{}", std::process::id());
    let mut spec = ContainerSpec::new(&name, "node:22-alpine");
    spec.binds = resolved.binds;
    spec.env = resolved.env;
    backend.create(&spec).await.unwrap();
    let out = backend
        .exec_collect(&name, &ExecSpec::new(["hello-tool"]))
        .await
        .unwrap();
    // Read-only: writes fail.
    let ro = backend
        .exec_collect(&name, &ExecSpec::new(["touch", "/deps/hello/x"]))
        .await
        .unwrap();
    backend.remove(&name).await.unwrap();
    assert_eq!(out.stdout_str(), "hello from hello-tool 1.0\n", "{}", out.stderr_str());
    assert!(!ro.success());

    // Changing the lockfile makes the build stale.
    std::fs::write(repo.path().join("deps.lock"), "hello-tool 2.0\n").unwrap();
    let second = builder.build(&manifest, repo.path(), "node:22-alpine").await.unwrap();
    assert!(second.built);
    assert_ne!(second.identity, first.identity);
    builder
        .prune("hello", std::slice::from_ref(&second.identity))
        .await
        .unwrap();
    assert!(builder.last_log("hello").is_some(), "prune keeps the last log");
    // stdout and stderr interleave in arrival order; compare as a set.
    let mut streamed = lines.into_inner().unwrap();
    streamed.sort();
    assert_eq!(streamed, vec!["building", "partial", "warn"]);

    // A failing build keeps its log and reports it.
    let mut failing = manifest.clone();
    failing.build.command = "echo about to fail; exit 7".into();
    let err = builder
        .build(&failing, repo.path(), "node:22-alpine")
        .await
        .unwrap_err();
    assert!(err.to_string().contains("exit Some(7)"), "{err}");
    assert!(err.to_string().contains("about to fail"));
    assert_eq!(builder.last_log("hello").unwrap(), "about to fail\n");
    assert!(!first.path.exists());
    assert!(second.path.exists());
}
