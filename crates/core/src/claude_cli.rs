//! The Claude CLI as an [`LlmProvider`].
//!
//! The CLI brings its own agent loop and tools, which execute wherever the CLI process runs, so
//! the harness launches it inside the sandbox container in non-interactive mode:
//!
//! ```text
//! claude -p <prompt> --output-format stream-json --verbose --include-partial-messages
//!        --permission-mode <mode> [--resume <session>] [--mcp-config <file> --strict-mcp-config]
//!        [--append-system-prompt <text>] [--model <model>]
//! ```
//!
//! Flags verified against Claude Code 2.1. Each line of stdout is one JSON event; unknown event
//! types are ignored so newer CLI versions keep working.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{AgentEvent, EventStream, LaunchSpec, LlmProvider, ProcessLauncher, ProcessOutput, TurnRequest};

/// Per-conversation (HOME is per conversation), so cancelling never signals another process.
const PID_FILE: &str = "${HOME:-/tmp}/.nucleus-claude.pid";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaudeCliConfig {
    /// CLI binary inside the container.
    pub binary: String,
    /// Working directory inside the container (the mounted worktree).
    pub workdir: String,
    /// `--permission-mode`. The container is the security boundary, so the default lets the
    /// CLI use its tools without prompting (there is no one to answer a prompt in `-p` mode).
    pub permission_mode: String,
    pub model: Option<String>,
    /// MCP config file inside the container, exposing harness tools to the CLI.
    pub mcp_config: Option<String>,
    /// Authentication and other environment, e.g. `ANTHROPIC_API_KEY` or
    /// `CLAUDE_CODE_OAUTH_TOKEN`. Each user brings their own.
    pub env: BTreeMap<String, String>,
    pub extra_args: Vec<String>,
}

impl Default for ClaudeCliConfig {
    fn default() -> Self {
        Self {
            binary: "claude".into(),
            workdir: "/workspace".into(),
            permission_mode: "bypassPermissions".into(),
            model: None,
            mcp_config: None,
            env: BTreeMap::new(),
            extra_args: Vec::new(),
        }
    }
}

/// Hosts the CLI needs to reach. Always allowed through the sandbox egress proxy.
pub const REQUIRED_HOSTS: &[&str] = &[
    "api.anthropic.com",
    "statsig.anthropic.com",
    "claude.ai",
    "platform.claude.com",
];

pub struct ClaudeCliProvider {
    launcher: Arc<dyn ProcessLauncher>,
    config: ClaudeCliConfig,
}

impl ClaudeCliProvider {
    pub fn new(launcher: Arc<dyn ProcessLauncher>, config: ClaudeCliConfig) -> Self {
        Self { launcher, config }
    }

    /// The command line for a turn, without the pid-file wrapper.
    pub fn argv(&self, req: &TurnRequest) -> Vec<String> {
        let c = &self.config;
        let mut argv: Vec<String> = vec![
            c.binary.clone(),
            "-p".into(),
            req.prompt.clone(),
            "--output-format".into(),
            "stream-json".into(),
            "--verbose".into(),
            "--include-partial-messages".into(),
            "--permission-mode".into(),
            c.permission_mode.clone(),
        ];
        if let Some(s) = &req.resume_session {
            argv.extend(["--resume".into(), s.clone()]);
        }
        if let Some(m) = &c.mcp_config {
            argv.extend(["--mcp-config".into(), m.clone(), "--strict-mcp-config".into()]);
        }
        if let Some(s) = req.system_append.as_ref().filter(|s| !s.is_empty()) {
            argv.extend(["--append-system-prompt".into(), s.clone()]);
        }
        if let Some(m) = req.model.as_ref().or(c.model.as_ref()) {
            argv.extend(["--model".into(), m.clone()]);
        }
        argv.extend(c.extra_args.iter().cloned());
        argv
    }
}

#[async_trait]
impl LlmProvider for ClaudeCliProvider {
    fn name(&self) -> &str {
        "claude-cli"
    }

    async fn start_turn(&self, request: TurnRequest) -> crate::Result<EventStream> {
        // Record the pid so `cancel` can signal the process from a separate exec.
        let mut argv = vec![
            "sh".to_string(),
            "-c".into(),
            format!("echo $$ > \"{PID_FILE}\"; exec \"$@\""),
            "nucleus-claude".into(),
        ];
        argv.extend(self.argv(&request));
        let process = self
            .launcher
            .launch(LaunchSpec {
                argv,
                env: self.config.env.clone(),
                workdir: Some(self.config.workdir.clone()),
            })
            .await?;

        let mut parser = StreamParser::default();
        let mut stderr_buf = LineBuffer::default();
        let output = process.output;
        let exit = process.exit;
        let events = async_stream(output, move |item| match item {
            Ok(ProcessOutput::Stdout(bytes)) => parser.feed(&bytes),
            Ok(ProcessOutput::Stderr(bytes)) => stderr_buf
                .push(&bytes)
                .into_iter()
                .filter(|l| !l.trim().is_empty())
                .map(|text| AgentEvent::Stderr { text })
                .collect(),
            Err(e) => vec![AgentEvent::Error { message: e.to_string() }],
        });
        let tail = futures::stream::once(async move {
            match exit.await {
                Ok(code) => AgentEvent::ProcessExited { code },
                Err(e) => AgentEvent::Error {
                    message: format!("waiting for claude: {e}"),
                },
            }
        });
        Ok(events.chain(tail).boxed())
    }

    async fn cancel(&self) -> crate::Result<()> {
        let p = self
            .launcher
            .launch(LaunchSpec {
                argv: vec![
                    "sh".into(),
                    "-c".into(),
                    format!("[ -f \"{PID_FILE}\" ] && kill -INT $(cat \"{PID_FILE}\")"),
                ],
                env: BTreeMap::new(),
                workdir: None,
            })
            .await?;
        p.output.for_each(|_| async {}).await;
        p.exit.await?;
        Ok(())
    }
}

fn async_stream<S, F>(input: S, mut f: F) -> impl futures::Stream<Item = AgentEvent> + Send
where
    S: futures::Stream<Item = crate::Result<ProcessOutput>> + Send,
    F: FnMut(crate::Result<ProcessOutput>) -> Vec<AgentEvent> + Send,
{
    input.flat_map(move |item| futures::stream::iter(f(item)))
}

/// Splits a byte stream into lines.
#[derive(Default)]
struct LineBuffer {
    buf: Vec<u8>,
}

impl LineBuffer {
    fn push(&mut self, bytes: &[u8]) -> Vec<String> {
        self.buf.extend_from_slice(bytes);
        let mut lines = Vec::new();
        while let Some(pos) = self.buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=pos).collect();
            lines.push(String::from_utf8_lossy(&line[..line.len() - 1]).into_owned());
        }
        lines
    }
}

/// Parses the CLI's `stream-json` output into [`AgentEvent`]s.
#[derive(Default)]
pub struct StreamParser {
    lines: LineBuffer,
}

impl StreamParser {
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<AgentEvent> {
        self.lines.push(bytes).iter().flat_map(|l| parse_line(l)).collect()
    }
}

/// Parse one line of `stream-json` output.
pub fn parse_line(line: &str) -> Vec<AgentEvent> {
    let line = line.trim();
    if line.is_empty() {
        return vec![];
    }
    let Ok(v) = serde_json::from_str::<Value>(line) else {
        return vec![AgentEvent::Stderr { text: line.to_string() }];
    };
    let s = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
    match v.get("type").and_then(Value::as_str) {
        Some("system") if v.get("subtype").and_then(Value::as_str) == Some("init") => {
            vec![AgentEvent::SessionStarted {
                session_id: s(&v, "session_id").unwrap_or_default(),
                model: s(&v, "model"),
                tools: v
                    .get("tools")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(|t| t.as_str().map(str::to_string)).collect())
                    .unwrap_or_default(),
            }]
        }
        Some("stream_event") => {
            let ev = &v["event"];
            if ev.get("type").and_then(Value::as_str) != Some("content_block_delta") {
                return vec![];
            }
            let delta = &ev["delta"];
            match delta.get("type").and_then(Value::as_str) {
                Some("text_delta") => vec![AgentEvent::TextDelta {
                    text: s(delta, "text").unwrap_or_default(),
                }],
                Some("thinking_delta") => {
                    vec![AgentEvent::ThinkingDelta {
                        text: s(delta, "thinking").unwrap_or_default(),
                    }]
                }
                _ => vec![],
            }
        }
        Some("assistant") => content(&v)
            .iter()
            .filter_map(|block| match block.get("type").and_then(Value::as_str) {
                Some("text") => Some(AgentEvent::AssistantText {
                    text: s(block, "text").unwrap_or_default(),
                }),
                Some("tool_use") => Some(AgentEvent::ToolUse {
                    id: s(block, "id").unwrap_or_default(),
                    name: s(block, "name").unwrap_or_default(),
                    input: block.get("input").cloned().unwrap_or(Value::Null),
                }),
                _ => None,
            })
            .collect(),
        Some("user") => content(&v)
            .iter()
            .filter(|b| b.get("type").and_then(Value::as_str) == Some("tool_result"))
            .map(|b| AgentEvent::ToolResult {
                tool_use_id: s(b, "tool_use_id").unwrap_or_default(),
                content: tool_result_text(b.get("content")),
                is_error: b.get("is_error").and_then(Value::as_bool).unwrap_or(false),
            })
            .collect(),
        Some("result") => vec![AgentEvent::TurnCompleted {
            is_error: v.get("is_error").and_then(Value::as_bool).unwrap_or(false),
            result: s(&v, "result"),
            session_id: s(&v, "session_id"),
            cost_usd: v.get("total_cost_usd").and_then(Value::as_f64),
            duration_ms: v.get("duration_ms").and_then(Value::as_u64),
            num_turns: v.get("num_turns").and_then(Value::as_u64).map(|n| n as u32),
        }],
        _ => vec![],
    }
}

fn content(v: &Value) -> Vec<Value> {
    v.pointer("/message/content")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn tool_result_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|p| p.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        Some(other) => other.to_string(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_real_event_shapes() {
        let init =
            r#"{"type":"system","subtype":"init","cwd":"/w","session_id":"s1","tools":["Bash","Edit"],"model":"m"}"#;
        assert_eq!(
            parse_line(init),
            vec![AgentEvent::SessionStarted {
                session_id: "s1".into(),
                model: Some("m".into()),
                tools: vec!["Bash".into(), "Edit".into()]
            }]
        );
        let delta = r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hi"}},"session_id":"s1"}"#;
        assert_eq!(parse_line(delta), vec![AgentEvent::TextDelta { text: "Hi".into() }]);
        let asst = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Hi there"},{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"ls"}}]},"session_id":"s1"}"#;
        assert_eq!(parse_line(asst).len(), 2);
        let user = r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":[{"type":"text","text":"a\nb"}],"is_error":false}]}}"#;
        assert_eq!(
            parse_line(user),
            vec![AgentEvent::ToolResult {
                tool_use_id: "t1".into(),
                content: "a\nb".into(),
                is_error: false
            }]
        );
        let result = r#"{"type":"result","subtype":"success","is_error":false,"duration_ms":10,"num_turns":2,"result":"done","session_id":"s1","total_cost_usd":0.01}"#;
        assert!(matches!(
            &parse_line(result)[0],
            AgentEvent::TurnCompleted { num_turns: Some(2), .. }
        ));
        assert!(parse_line(r#"{"type":"rate_limit_event"}"#).is_empty());
        assert!(parse_line(r#"{"type":"active_goal","value":null}"#).is_empty());
    }

    #[test]
    fn parser_handles_split_lines() {
        let mut p = StreamParser::default();
        assert!(
            p.feed(br#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text"#)
                .is_empty()
        );
        assert_eq!(
            p.feed(b"_delta\",\"text\":\"x\"}}}\n"),
            vec![AgentEvent::TextDelta { text: "x".into() }]
        );
    }
}
