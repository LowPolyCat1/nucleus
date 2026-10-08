use serde::{Deserialize, Serialize};

/// Everything a provider reports while a turn runs. Streamed to the UI as it happens.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    SessionStarted { session_id: String, model: Option<String>, tools: Vec<String> },
    /// Incremental assistant text.
    TextDelta { text: String },
    /// Incremental reasoning text.
    ThinkingDelta { text: String },
    /// A complete assistant text block. Supersedes the deltas streamed for it.
    AssistantText { text: String },
    ToolUse { id: String, name: String, input: serde_json::Value },
    ToolResult { tool_use_id: String, content: String, is_error: bool },
    TurnCompleted {
        is_error: bool,
        result: Option<String>,
        session_id: Option<String>,
        cost_usd: Option<f64>,
        duration_ms: Option<u64>,
        num_turns: Option<u32>,
    },
    /// Diagnostic output from the provider process.
    Stderr { text: String },
    Error { message: String },
    ProcessExited { code: Option<i64> },
}
