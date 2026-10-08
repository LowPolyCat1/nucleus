//! Proposals from the agent's outbox and the unified proposal API for the UI.

use nucleus_promotion::{FileChange, LibraryKind, NewProposal, Proposal};
use nucleus_templates::TemplateManifest;
use nucleus_vcs::{CommitInfo, FileDiff};
use serde::{Deserialize, Serialize};

use crate::{Conversation, Harness, HarnessEvent, OUTBOX_MOUNT, Result};

/// A proposal in any library.
pub type AnyProposal = Proposal;

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum OutboxItem {
    Skill { content: String, rationale: String },
    Tool { name: String, rationale: String },
    Template { manifest: String, rationale: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposalDetail {
    pub proposal: Proposal,
    pub diff: Vec<FileDiff>,
}

impl Harness {
    /// Turn outbox entries written by the in-container MCP server into proposals. Each entry is
    /// processed once; failures are reported as events and the entry is moved aside.
    pub(crate) async fn process_outbox(&self, conv: &Conversation) {
        let (_, outbox, _) = self.conversation_dirs(&conv.id);
        let dir = outbox.join("proposals");
        let Ok(rd) = std::fs::read_dir(&dir) else { return };
        let mut files: Vec<_> = rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "json")).collect();
        files.sort();
        let done = outbox.join("processed");
        std::fs::create_dir_all(&done).ok();
        for file in files {
            let parsed = std::fs::read_to_string(&file)
                .map_err(anyhow::Error::from)
                .and_then(|t| serde_json::from_str::<OutboxItem>(&t).map_err(anyhow::Error::from));
            let kind = match &parsed {
                Ok(OutboxItem::Skill { .. }) | Err(_) => LibraryKind::Skills,
                Ok(OutboxItem::Tool { .. }) => LibraryKind::Tools,
                Ok(OutboxItem::Template { .. }) => LibraryKind::Templates,
            };
            let result = match parsed {
                Ok(item) => self.propose_from_outbox(conv, item).await,
                Err(e) => Err(e.context("unreadable outbox entry")),
            };
            match result {
                Ok(proposal) => self.emit(HarnessEvent::ProposalCreated { proposal }),
                Err(e) => self.emit(HarnessEvent::ProposalFailed { conversation_id: conv.id.clone(), kind, error: format!("{e:#}") }),
            }
            if let Some(name) = file.file_name() {
                std::fs::rename(&file, done.join(name)).ok();
            }
        }
    }

    async fn propose_from_outbox(&self, conv: &Conversation, item: OutboxItem) -> Result<Proposal> {
        let source = Some(conv.id.clone());
        match item {
            OutboxItem::Skill { content, rationale } => self.skills.propose_upsert(&content, &rationale, source).await,
            OutboxItem::Tool { name, rationale } => {
                nucleus_skills::validate_name(&name.replace('_', "-"))?;
                let (_, outbox, _) = self.conversation_dirs(&conv.id);
                let report = nucleus_tools::promote_candidate(
                    &self.tools,
                    self.backend.as_ref(),
                    &conv.container,
                    &outbox.join("tools").join(&name),
                    &format!("{OUTBOX_MOUNT}/tools/{name}"),
                    &rationale,
                    source,
                )
                .await?;
                Ok(report.proposal)
            }
            OutboxItem::Template { manifest, rationale } => {
                let m = TemplateManifest::parse(&manifest)?;
                let exists = self.templates.root().join(&m.name).exists();
                self.templates
                    .propose(NewProposal {
                        title: format!("{} template {}", if exists { "Update" } else { "Add" }, m.name),
                        rationale,
                        changes: vec![FileChange { path: format!("{}/{}", m.name, nucleus_templates::MANIFEST_FILE), content: Some(manifest), executable: false }],
                        source,
                    })
                    .await
            }
        }
    }

    /// Pending proposals across all libraries, newest first.
    pub async fn proposals(&self) -> Result<Vec<Proposal>> {
        let mut all = Vec::new();
        for kind in [LibraryKind::Skills, LibraryKind::Tools, LibraryKind::Templates] {
            all.extend(self.library(kind).proposals().await?);
        }
        all.sort_by(|a, b| b.created.cmp(&a.created));
        Ok(all)
    }

    pub async fn proposal(&self, kind: LibraryKind, id: &str) -> Result<ProposalDetail> {
        let lib = self.library(kind);
        Ok(ProposalDetail { proposal: lib.get(id).await?, diff: lib.diff(id).await? })
    }

    /// Approve a proposal. Running containers see approved skills and tools immediately
    /// (read-only mounts of the library); the tool index is refreshed for every conversation.
    pub async fn approve(&self, kind: LibraryKind, id: &str) -> Result<String> {
        let commit = self.library(kind).approve(id).await?;
        self.refresh_support().await;
        Ok(commit)
    }

    pub async fn reject(&self, kind: LibraryKind, id: &str) -> Result<()> {
        self.library(kind).reject(id).await
    }

    pub async fn history(&self, kind: LibraryKind, limit: usize) -> Result<Vec<CommitInfo>> {
        self.library(kind).history(limit).await
    }

    pub async fn revert(&self, kind: LibraryKind, commit: &str) -> Result<String> {
        let c = self.library(kind).revert(commit).await?;
        self.refresh_support().await;
        Ok(c)
    }

    async fn refresh_support(&self) {
        let ids: Vec<String> = self.state.lock().await.conversations.iter().map(|c| c.id.clone()).collect();
        for id in ids {
            self.write_support(&id).ok();
        }
    }
}
