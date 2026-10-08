use nucleus_core::AgentEvent;
use nucleus_promotion::{LibraryKind, Proposal};
use serde::{Deserialize, Serialize};

use crate::ConversationStatus;

/// Events pushed to the UI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HarnessEvent {
    Agent {
        conversation_id: String,
        event: AgentEvent,
    },
    Status {
        conversation_id: String,
        status: ConversationStatus,
    },
    Committed {
        conversation_id: String,
        commit: String,
    },
    ProposalCreated {
        proposal: Proposal,
    },
    ProposalFailed {
        conversation_id: String,
        kind: LibraryKind,
        error: String,
    },
    Progress {
        message: String,
    },
    /// One line of output from a running template build.
    BuildOutput {
        template: String,
        line: String,
    },
}

pub type EventSink = std::sync::Arc<dyn Fn(HarnessEvent) + Send + Sync>;
