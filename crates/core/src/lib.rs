//! Agent loop, message and event types, and LLM providers.
//!
//! Providers implement [`LlmProvider`] and stream [`AgentEvent`]s. The first provider is the
//! Claude CLI ([`claude_cli::ClaudeCliProvider`]), which brings its own tool loop and runs inside
//! the sandbox through a [`ProcessLauncher`].

mod agent;
pub mod claude_cli;
mod event;
mod launcher;
mod provider;

pub use agent::{Agent, TranscriptEntry, TurnSummary};
pub use event::AgentEvent;
pub use launcher::*;
pub use provider::{EventStream, LlmProvider, TurnRequest};

pub type Result<T, E = anyhow::Error> = std::result::Result<T, E>;
