use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

use nucleus_promotion::{Library, LibraryKind};
use nucleus_sandbox::{BindMount, BollardBackend, ContainerSpec, MountMode, SandboxBackend};
use nucleus_tools::*;
use serde_json::{Value, json};

fn write_tool(dir: &std::path::Path, name: &str, test: &str) {
    let d = dir.join(name);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(
        d.join("tool.toml"),
        format!(
            "name = \"{name}\"\ndescription = \"Greets someone\"\nrun = \"sh greet.sh\"\ntest = \"{test}\"\n[input_schema]\ntype = \"object\"\nproperties = {{ who = {{ type = \"string\" }} }}\n"
        ),
    )
    .unwrap();
    std::fs::write(d.join("greet.sh"), "read input; echo \"hello $(echo \"$input\" | sed 's/.*\"who\":\"\\([^\"]*\\)\".*/\\1/')\"\n").unwrap();
}

#[test]
fn registry_and_index() {
    let dir = tempfile::tempdir().unwrap();
    write_tool(dir.path(), "greet", "true");
    std::fs::create_dir_all(dir.path().join("broken")).unwrap();
    std::fs::write(dir.path().join("broken/tool.toml"), "name = \"Bad Name\"").unwrap();
    let tools = load_registry(dir.path());
    assert_eq!(tools.len(), 1);
    let index = tools_index(&tools);
    assert_eq!(index[0]["dir"], "/nucleus/tools/greet");
    assert_eq!(
        index[0]["input_schema"]["properties"]["who"]["type"],
        "string"
    );
}

/// Talks to the MCP server over stdio with node on the host.
#[test]
fn mcp_server_protocol() {
    if Command::new("node").arg("--version").output().is_err() {
        eprintln!("skipping: node not installed");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let support = dir.path().join("support");
    let outbox = dir.path().join("outbox");
    let tools_dir = dir.path().join("tools");
    std::fs::create_dir_all(&support).unwrap();
    write_tool(&tools_dir, "greet", "true");
    let mut index = tools_index(&load_registry(&tools_dir));
    index[0]["dir"] = json!(tools_dir.join("greet"));
    std::fs::write(support.join("tools.json"), index.to_string()).unwrap();
    std::fs::write(support.join("mcp-server.js"), MCP_SERVER_JS).unwrap();

    let mut child = Command::new("node")
        .arg(support.join("mcp-server.js"))
        .env("NUCLEUS_SUPPORT_DIR", &support)
        .env("NUCLEUS_OUTBOX_DIR", &outbox)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    let mut call = |id: u64, method: &str, params: Value| -> Value {
        writeln!(
            stdin,
            "{}",
            json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
        )
        .unwrap();
        serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap()
    };
    let init = call(
        1,
        "initialize",
        json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "t"}}),
    );
    assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
    let list = call(2, "tools/list", json!({}));
    let names: Vec<_> = list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        names,
        ["propose_skill", "propose_tool", "propose_template", "greet"]
    );
    let res = call(
        3,
        "tools/call",
        json!({"name": "greet", "arguments": {"who": "ada"}}),
    );
    assert_eq!(res["result"]["content"][0]["text"], "hello ada\n");
    assert_eq!(res["result"]["isError"], false);
    let res = call(
        4,
        "tools/call",
        json!({"name": "propose_skill", "arguments": {"content": "---", "rationale": "r"}}),
    );
    assert!(
        res["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("recorded")
    );
    let proposals: Vec<_> = std::fs::read_dir(outbox.join("proposals"))
        .unwrap()
        .collect();
    assert_eq!(proposals.len(), 1);
    let res = call(
        5,
        "tools/call",
        json!({"name": "propose_tool", "arguments": {"name": "missing", "rationale": "r"}}),
    );
    assert_eq!(res["result"]["isError"], true);
    drop(stdin);
    child.wait().unwrap();
}

/// Runs a tool and a candidate test in a real container. Run with `NUCLEUS_ENGINE_TESTS=1`.
#[tokio::test]
async fn sandboxed_call_and_promotion() {
    if std::env::var("NUCLEUS_ENGINE_TESTS").is_err() {
        return;
    }
    let backend = BollardBackend::connect_default(std::env::temp_dir().join("nucleus-tools-test"))
        .await
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let lib = Library::open_or_init(dir.path().join("tools-lib"), LibraryKind::Tools)
        .await
        .unwrap();
    let outbox = dir.path().join("outbox");
    write_tool(
        &outbox.join("tools"),
        "greet",
        "echo '{\\\"who\\\":\\\"x\\\"}' | sh greet.sh | grep -q 'hello x'",
    );
    write_tool(&outbox.join("tools"), "failing", "exit 1");

    let name = format!("nucleus-tools-test-{}", std::process::id());
    let mut spec = ContainerSpec::new(&name, "node:22-alpine");
    spec.binds.push(BindMount {
        source: outbox.clone(),
        target: "/nucleus/outbox".into(),
        mode: MountMode::ReadWrite,
    });
    spec.binds.push(BindMount {
        source: lib.root().into(),
        target: TOOLS_MOUNT.into(),
        mode: MountMode::ReadOnly,
    });
    backend.create(&spec).await.unwrap();

    let ok = promote_candidate(
        &lib,
        &backend,
        &name,
        &outbox.join("tools/greet"),
        "/nucleus/outbox/tools/greet",
        "useful",
        Some("c1".into()),
    )
    .await;
    let bad = promote_candidate(
        &lib,
        &backend,
        &name,
        &outbox.join("tools/failing"),
        "/nucleus/outbox/tools/failing",
        "x",
        None,
    )
    .await;
    let report = ok.unwrap();
    assert!(bad.unwrap_err().to_string().contains("failed"));
    lib.approve(&report.proposal.id).await.unwrap();

    let tools = load_registry(lib.root());
    let out = tools[0]
        .call(&backend, &name, json!({"who": "grace"}))
        .await
        .unwrap();
    backend.remove(&name).await.unwrap();
    assert_eq!(
        out,
        ToolOutput {
            content: "hello grace\n".into(),
            is_error: false
        }
    );
}
