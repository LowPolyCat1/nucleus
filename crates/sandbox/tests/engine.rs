//! Integration tests against a real engine. Run with `NUCLEUS_ENGINE_TESTS=1`.

use nucleus_sandbox::*;
use tokio::io::AsyncWriteExt;

const IMAGE: &str = "node:22-alpine";

async fn backend() -> Option<BollardBackend> {
    if std::env::var("NUCLEUS_ENGINE_TESTS").is_err() {
        eprintln!("skipping: set NUCLEUS_ENGINE_TESTS=1");
        return None;
    }
    let dir = std::env::temp_dir().join("nucleus-sandbox-test-support");
    Some(BollardBackend::connect_default(dir).await.expect("engine"))
}

#[tokio::test]
async fn isolated_exec_mounts_and_stdin() {
    let Some(b) = backend().await else { return };
    let ws = tempfile::tempdir().unwrap();
    std::fs::write(ws.path().join("hello.txt"), "from host").unwrap();
    let name = format!("nucleus-test-iso-{}", std::process::id());
    let mut spec = ContainerSpec::new(&name, IMAGE);
    spec.binds.push(BindMount {
        source: ws.path().into(),
        target: "/workspace".into(),
        mode: MountMode::ReadWrite,
    });
    spec.workdir = Some("/workspace".into());
    spec.labels.insert("nucleus.conversation".into(), "test".into());
    b.create(&spec).await.unwrap();

    let res = b
        .exec_collect(
            &name,
            &ExecSpec::new(["sh", "-c", "cat hello.txt; echo out > new.txt; echo err >&2; exit 3"]),
        )
        .await
        .unwrap();
    assert_eq!(res.stdout_str(), "from host");
    assert_eq!(res.stderr_str(), "err\n");
    assert_eq!(res.exit_code, Some(3));
    assert_eq!(std::fs::read_to_string(ws.path().join("new.txt")).unwrap(), "out\n");

    // No network at all.
    let res = b
        .exec_collect(&name, &ExecSpec::new(["sh", "-c", "ip -o link | grep -vc ': lo:'"]))
        .await
        .unwrap();
    assert_eq!(res.stdout_str().trim(), "0");

    let mut spec = ExecSpec::new(["cat"]);
    spec.stdin = true;
    let mut h = b.exec(&name, &spec).await.unwrap();
    let mut stdin = h.stdin.take().unwrap();
    stdin.write_all(b"piped").await.unwrap();
    stdin.shutdown().await.unwrap();
    drop(stdin);
    use futures::StreamExt;
    let mut out = Vec::new();
    while let Some(c) = h.output.next().await {
        if let ExecChunk::Stdout(b) = c.unwrap() {
            out.extend(b)
        }
    }
    assert_eq!(out, b"piped");
    assert_eq!(h.wait().await.unwrap(), Some(0));

    assert!(
        b.list(Some(("nucleus.conversation", "test")))
            .await
            .unwrap()
            .iter()
            .any(|c| c.name == name)
    );
    b.remove(&name).await.unwrap();
    b.remove(&name).await.unwrap();
    assert!(!b.list(None).await.unwrap().iter().any(|c| c.name == name));
}

#[tokio::test]
async fn proxied_network_denies_unlisted_hosts() {
    let Some(b) = backend().await else { return };
    let name = format!("nucleus-test-proxy-{}", std::process::id());
    let mut spec = ContainerSpec::new(&name, IMAGE);
    spec.required_hosts = vec!["api.anthropic.com".into()];
    b.create(&spec).await.unwrap();
    // Give the proxy a moment to listen.
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
    let script = r#"
      const http = require('http');
      http.get({host: 'egress', port: 3128, path: 'http://denied.example/', headers: {host: 'denied.example'}}, r => {
        console.log(r.statusCode); process.exit(0);
      }).on('error', e => { console.log('ERR ' + e.message); process.exit(1); });
    "#;
    let res = b
        .exec_collect(&name, &ExecSpec::new(["node", "-e", script]))
        .await
        .unwrap();
    assert_eq!(res.stdout_str().trim(), "403", "{}", res.stderr_str());
    // Direct connections bypassing the proxy fail: the network is internal.
    let direct = r#"require('net').connect(80, '1.1.1.1').on('connect', () => {console.log('open'); process.exit(0)}).on('error', () => {console.log('blocked'); process.exit(0)}); setTimeout(() => {console.log('blocked'); process.exit(0)}, 3000)"#;
    let res = b
        .exec_collect(&name, &ExecSpec::new(["node", "-e", direct]))
        .await
        .unwrap();
    assert_eq!(res.stdout_str().trim(), "blocked");
    b.remove(&name).await.unwrap();
}
