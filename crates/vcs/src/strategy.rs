//! Branching strategy, kept separate from the generic [`crate::Vcs`] layer.
//!
//! The default [`NamespacedStrategy`] uses three namespaces:
//! - `origin/*`: remote-tracking branches,
//! - `local/*`: the user's own branches (every local branch outside `agent/`),
//! - `agent/*`: one branch per agent conversation, `agent/<conversation-id>`.

use crate::BranchKind;

pub trait BranchStrategy: Send + Sync {
    /// Classify a full ref name, or `None` if it is not a branch the harness shows.
    fn classify(&self, full_ref: &str) -> Option<BranchKind>;
    /// Short branch name for a conversation.
    fn agent_branch(&self, conversation_id: &str) -> String;
    /// The conversation id if `branch` (short name) is an agent branch.
    fn conversation_of(&self, branch: &str) -> Option<String>;
    /// Suggested short name for keeping a copy of an agent branch.
    fn keep_copy_name(&self, conversation_id: &str) -> String;
}

#[derive(Debug, Clone, Default)]
pub struct NamespacedStrategy;

const AGENT_PREFIX: &str = "agent/";

impl BranchStrategy for NamespacedStrategy {
    fn classify(&self, full_ref: &str) -> Option<BranchKind> {
        if let Some(short) = full_ref.strip_prefix("refs/heads/") {
            Some(if short.starts_with(AGENT_PREFIX) {
                BranchKind::Agent
            } else {
                BranchKind::Local
            })
        } else if let Some(short) = full_ref.strip_prefix("refs/remotes/") {
            // `origin/HEAD` is a symbolic pointer, not a branch.
            (!short.ends_with("/HEAD")).then_some(BranchKind::Origin)
        } else {
            None
        }
    }

    fn agent_branch(&self, conversation_id: &str) -> String {
        format!("{AGENT_PREFIX}{conversation_id}")
    }

    fn conversation_of(&self, branch: &str) -> Option<String> {
        branch
            .strip_prefix(AGENT_PREFIX)
            .filter(|id| !id.is_empty())
            .map(str::to_string)
    }

    fn keep_copy_name(&self, conversation_id: &str) -> String {
        format!("local/{conversation_id}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_namespaces() {
        let s = NamespacedStrategy;
        assert_eq!(s.classify("refs/heads/main"), Some(BranchKind::Local));
        assert_eq!(s.classify("refs/heads/local/x"), Some(BranchKind::Local));
        assert_eq!(s.classify("refs/heads/agent/42"), Some(BranchKind::Agent));
        assert_eq!(s.classify("refs/remotes/origin/main"), Some(BranchKind::Origin));
        assert_eq!(s.classify("refs/remotes/origin/HEAD"), None);
        assert_eq!(s.classify("refs/tags/v1"), None);
        assert_eq!(s.conversation_of("agent/42").as_deref(), Some("42"));
        assert_eq!(s.conversation_of("main"), None);
    }
}
