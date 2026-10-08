use async_trait::async_trait;
use futures::stream::BoxStream;
use serde::{Deserialize, Serialize};

use crate::AgentEvent;

pub type EventStream = BoxStream<'static, AgentEvent>;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnRequest {
    pub prompt: String,
    /// Provider session to continue, from a previous [`AgentEvent::SessionStarted`].
    pub resume_session: Option<String>,
    /// Extra system prompt text, e.g. the skills index.
    pub system_append: Option<String>,
    pub model: Option<String>,
}

/// A language model provider. One instance serves one conversation.
#[async_trait]
pub trait LlmProvider: Send + Sync {
    fn name(&self) -> &str;

    /// Start a turn. The stream ends after [`AgentEvent::ProcessExited`] or an error.
    async fn start_turn(&self, request: TurnRequest) -> crate::Result<EventStream>;

    /// Ask the running turn to stop.
    async fn cancel(&self) -> crate::Result<()>;
}
