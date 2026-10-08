use std::sync::Arc;

use futures::StreamExt;
use serde::{Deserialize, Serialize};

use crate::{AgentEvent, LlmProvider, TurnRequest};

/// One entry of a conversation transcript as shown in the UI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "snake_case")]
pub enum TranscriptEntry {
    User {
        text: String,
    },
    Assistant {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    ToolResult {
        tool_use_id: String,
        content: String,
        is_error: bool,
    },
    Error {
        message: String,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TurnSummary {
    pub session_id: Option<String>,
    pub is_error: bool,
    pub cost_usd: Option<f64>,
    pub exit_code: Option<i64>,
    pub entries: Vec<TranscriptEntry>,
}

/// The agent loop for one conversation: runs turns on a provider, keeps the provider session
/// for continuity and builds the transcript.
pub struct Agent {
    provider: Arc<dyn LlmProvider>,
    session_id: Option<String>,
    system_append: Option<String>,
    model: Option<String>,
}

impl Agent {
    pub fn new(provider: Arc<dyn LlmProvider>) -> Self {
        Self {
            provider,
            session_id: None,
            system_append: None,
            model: None,
        }
    }

    pub fn with_session(mut self, session_id: Option<String>) -> Self {
        self.session_id = session_id;
        self
    }

    pub fn set_system_append(&mut self, text: Option<String>) {
        self.system_append = text;
    }

    pub fn set_model(&mut self, model: Option<String>) {
        self.model = model;
    }

    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    pub fn provider(&self) -> &Arc<dyn LlmProvider> {
        &self.provider
    }

    /// Run one user turn. Every event is passed to `on_event` as it arrives.
    pub async fn run_turn(
        &mut self,
        prompt: &str,
        mut on_event: impl FnMut(&AgentEvent) + Send,
    ) -> crate::Result<TurnSummary> {
        let mut stream = self
            .provider
            .start_turn(TurnRequest {
                prompt: prompt.to_string(),
                resume_session: self.session_id.clone(),
                system_append: self.system_append.clone(),
                model: self.model.clone(),
            })
            .await?;
        let mut summary = TurnSummary {
            entries: vec![TranscriptEntry::User {
                text: prompt.to_string(),
            }],
            ..Default::default()
        };
        let mut completed = false;
        while let Some(event) = stream.next().await {
            on_event(&event);
            match event {
                AgentEvent::SessionStarted { session_id, .. } if !session_id.is_empty() => {
                    self.session_id = Some(session_id);
                }
                AgentEvent::AssistantText { text } => {
                    summary.entries.push(TranscriptEntry::Assistant { text })
                }
                AgentEvent::ToolUse { id, name, input } => summary
                    .entries
                    .push(TranscriptEntry::ToolUse { id, name, input }),
                AgentEvent::ToolResult {
                    tool_use_id,
                    content,
                    is_error,
                } => summary.entries.push(TranscriptEntry::ToolResult {
                    tool_use_id,
                    content,
                    is_error,
                }),
                AgentEvent::TurnCompleted {
                    is_error,
                    session_id,
                    cost_usd,
                    ..
                } => {
                    completed = true;
                    summary.is_error = is_error;
                    summary.cost_usd = cost_usd;
                    if let Some(s) = session_id {
                        self.session_id = Some(s);
                    }
                }
                AgentEvent::Error { message } => {
                    summary.is_error = true;
                    summary.entries.push(TranscriptEntry::Error { message });
                }
                AgentEvent::ProcessExited { code } => summary.exit_code = code,
                _ => {}
            }
        }
        if !completed {
            summary.is_error = true;
            let message = format!(
                "{} exited without completing the turn",
                self.provider.name()
            );
            on_event(&AgentEvent::Error {
                message: message.clone(),
            });
            summary.entries.push(TranscriptEntry::Error { message });
        }
        summary.session_id = self.session_id.clone();
        Ok(summary)
    }
}
