//! Drives the Claude CLI provider against a fake `claude` script on the host.

use std::sync::Arc;

use nucleus_core::claude_cli::{ClaudeCliConfig, ClaudeCliProvider};
use nucleus_core::{Agent, AgentEvent, LocalLauncher, TranscriptEntry};

#[tokio::test]
async fn agent_runs_turns_and_resumes_session() {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("claude");
    std::fs::write(
        &script,
        r#"#!/bin/sh
echo "$@" > "$(dirname "$0")/args"
echo '{"type":"system","subtype":"init","session_id":"sess-1","tools":["Bash"],"model":"m"}'
echo '{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"Wor"}}}'
echo '{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"king"}}}'
echo '{"type":"assistant","message":{"content":[{"type":"text","text":"Working"},{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"ls"}}]}}'
echo '{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"file.txt","is_error":false}]}}'
echo "progress" >&2
echo '{"type":"result","subtype":"success","is_error":false,"result":"done","session_id":"sess-1","total_cost_usd":0.5}'
"#,
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

    let config = ClaudeCliConfig {
        binary: script.to_str().unwrap().into(),
        workdir: dir.path().to_str().unwrap().into(),
        ..Default::default()
    };
    let provider = Arc::new(ClaudeCliProvider::new(Arc::new(LocalLauncher), config));
    let mut agent = Agent::new(provider);
    let mut events = Vec::new();
    let summary = agent
        .run_turn("list files", |e| events.push(e.clone()))
        .await
        .unwrap();

    assert_eq!(summary.session_id.as_deref(), Some("sess-1"));
    assert!(!summary.is_error);
    assert_eq!(summary.exit_code, Some(0));
    assert_eq!(summary.cost_usd, Some(0.5));
    assert_eq!(summary.entries.len(), 4);
    assert!(matches!(&summary.entries[2], TranscriptEntry::ToolUse { name, .. } if name == "Bash"));
    assert!(events.contains(&AgentEvent::TextDelta { text: "Wor".into() }));
    assert!(events.contains(&AgentEvent::Stderr {
        text: "progress".into()
    }));

    agent.run_turn("again", |_| {}).await.unwrap();
    let args = std::fs::read_to_string(dir.path().join("args")).unwrap();
    assert!(args.contains("--resume sess-1"), "{args}");
    assert!(args.contains("--output-format stream-json --verbose --include-partial-messages"));
}

#[tokio::test]
async fn missing_result_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let config = ClaudeCliConfig {
        binary: "false".into(),
        workdir: dir.path().to_str().unwrap().into(),
        ..Default::default()
    };
    let mut agent = Agent::new(Arc::new(ClaudeCliProvider::new(
        Arc::new(LocalLauncher),
        config,
    )));
    let summary = agent.run_turn("x", |_| {}).await.unwrap();
    assert!(summary.is_error);
    assert_eq!(summary.exit_code, Some(1));
}
