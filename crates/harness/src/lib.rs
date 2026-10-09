//! Orchestration: ties the agent loop, sandbox, version control, templates, skills and tools
//! together. The Tauri app is a thin shell over [`Harness`].
//!
//! Lifecycle: an `agent/<conversation-id>` branch, its worktree and its container exist exactly
//! as long as the conversation. On startup [`Harness::cleanup_orphans`] removes leftovers from
//! crashes or failed deletes.

mod events;
mod image;
mod launcher;
mod proposals;
mod sandbox_git;
mod state;
mod update;

pub use events::{EventSink, HarnessEvent};
pub use image::{AGENT_CONTAINERFILE, BUILD_CA_FILE, build_agent_image, build_image};
pub use launcher::SandboxLauncher;
pub use proposals::{AnyProposal, ProposalDetail};
pub use state::*;

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, bail};
use nucleus_core::claude_cli::{ClaudeCliConfig, ClaudeCliProvider, REQUIRED_HOSTS};
use nucleus_core::{Agent, AgentEvent, LlmProvider, TranscriptEntry, TurnSummary};
use nucleus_promotion::{Library, LibraryKind};
use nucleus_sandbox::{BindMount, ContainerSpec, MountMode, NetworkPolicy, SandboxBackend, caches, current_user};
use nucleus_skills::{Outcome, SkillLibrary};
use nucleus_templates::{TemplateBuilder, TemplateManifest, TemplateMount};
use nucleus_vcs::strategy::BranchStrategy;
use nucleus_vcs::{BranchInfo, CommitInfo, FileDiff, GixVcs, MergeOutcome, NamespacedStrategy, Vcs, exclude};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

pub type Result<T, E = anyhow::Error> = std::result::Result<T, E>;

const WORKSPACE_MOUNT: &str = "/workspace";
const HOME_MOUNT: &str = "/home/agent";
const SUPPORT_MOUNT: &str = "/nucleus/support";
const OUTBOX_MOUNT: &str = "/nucleus/outbox";
const CONVERSATION_LABEL: &str = "nucleus.conversation";

/// Directory layout under the data directory.
#[derive(Debug, Clone)]
pub struct Paths {
    pub root: PathBuf,
}

impl Paths {
    pub fn state(&self) -> PathBuf {
        self.root.join("state.json")
    }
    pub fn library(&self, kind: LibraryKind) -> PathBuf {
        self.root.join("libraries").join(kind.as_str())
    }
    pub fn skill_usage(&self) -> PathBuf {
        self.root.join("skill-usage.json")
    }
    pub fn template_builds(&self) -> PathBuf {
        self.root.join("template-builds")
    }
    pub fn worktrees(&self) -> PathBuf {
        self.root.join("worktrees")
    }
    pub fn conversation(&self, id: &str) -> PathBuf {
        self.root.join("conversations").join(id)
    }
    pub fn engine_support(&self) -> PathBuf {
        self.root.join("engine-support")
    }
}

/// What to do with an agent branch's unique commits when deleting its conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum DeleteMode {
    /// Refuse if the branch has commits not reachable from any local or origin branch.
    Check,
    /// Delete anyway; the commits become unreachable and are garbage collected later.
    Discard,
    /// Merge into this local branch first.
    MergeInto { branch: String },
    /// Keep a copy of the branch under this name first.
    KeepCopy { branch: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum DeleteOutcome {
    Deleted,
    /// Nothing was deleted; these commits would be lost.
    NeedsConfirmation {
        unmerged: Vec<CommitInfo>,
    },
    /// Nothing was deleted; merging failed.
    MergeConflicts {
        paths: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateStatus {
    pub name: String,
    pub description: String,
    /// Built for the current lockfiles and image.
    pub fresh: bool,
    pub identity: Option<String>,
    pub error: Option<String>,
}

/// The egress proxy's view of a conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EgressLog {
    /// `isolated` (no network), `proxied` (allowlist through the proxy) or `open` (full access).
    pub mode: String,
    /// Hosts the proxy lets through.
    pub allowed: Vec<String>,
    pub entries: Vec<nucleus_sandbox::EgressEntry>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CleanupReport {
    pub containers: Vec<String>,
    pub worktrees: Vec<PathBuf>,
    pub branches: Vec<String>,
    /// Items that could not be removed; cleanup continues past them.
    pub errors: Vec<String>,
}

pub struct Harness {
    paths: Paths,
    backend: Arc<dyn SandboxBackend>,
    state: Mutex<State>,
    skills: SkillLibrary,
    tools: Library,
    templates: Library,
    strategy: Arc<dyn BranchStrategy>,
    sink: EventSink,
    running: Mutex<HashMap<String, Arc<dyn LlmProvider>>>,
    /// Serialises turns, merges and deletes per conversation.
    locks: Mutex<HashMap<String, Arc<Mutex<()>>>>,
}

impl Harness {
    pub async fn open(data_dir: impl Into<PathBuf>, backend: Arc<dyn SandboxBackend>, sink: EventSink) -> Result<Self> {
        let paths = Paths { root: data_dir.into() };
        std::fs::create_dir_all(&paths.root)?;
        let mut state = State::load(&paths.state())?;
        // A crash mid-turn leaves conversations marked running.
        for c in &mut state.conversations {
            if c.status == ConversationStatus::Running {
                c.status = ConversationStatus::Idle;
            }
        }
        Ok(Self {
            skills: SkillLibrary::open(paths.library(LibraryKind::Skills), paths.skill_usage()).await?,
            tools: Library::open_or_init(paths.library(LibraryKind::Tools), LibraryKind::Tools).await?,
            templates: Library::open_or_init(paths.library(LibraryKind::Templates), LibraryKind::Templates).await?,
            paths,
            backend,
            state: Mutex::new(state),
            strategy: Arc::new(NamespacedStrategy),
            sink,
            running: Mutex::new(HashMap::new()),
            locks: Mutex::new(HashMap::new()),
        })
    }

    pub fn paths(&self) -> &Paths {
        &self.paths
    }

    pub fn backend(&self) -> &Arc<dyn SandboxBackend> {
        &self.backend
    }

    pub fn skills(&self) -> &SkillLibrary {
        &self.skills
    }

    pub fn library(&self, kind: LibraryKind) -> &Library {
        match kind {
            LibraryKind::Skills => self.skills.library(),
            LibraryKind::Tools => &self.tools,
            LibraryKind::Templates => &self.templates,
        }
    }

    fn emit(&self, e: HarnessEvent) {
        (self.sink)(e)
    }

    async fn mutate<T>(&self, f: impl FnOnce(&mut State) -> Result<T>) -> Result<T> {
        let mut state = self.state.lock().await;
        let out = f(&mut state)?;
        state.save(&self.paths.state())?;
        Ok(out)
    }

    pub async fn snapshot(&self) -> State {
        self.state.lock().await.clone()
    }

    async fn conversation_lock(&self, id: &str) -> Arc<Mutex<()>> {
        self.locks.lock().await.entry(id.to_string()).or_default().clone()
    }

    // ---- settings ----------------------------------------------------------------------

    pub async fn settings(&self) -> Settings {
        self.state.lock().await.settings.clone()
    }

    pub async fn update_settings(&self, settings: Settings) -> Result<()> {
        settings.validate()?;
        self.mutate(|s| {
            s.settings = settings;
            Ok(())
        })
        .await
    }

    // ---- workspaces --------------------------------------------------------------------

    pub async fn add_workspace(&self, repo: &Path, name: Option<String>) -> Result<Workspace> {
        let vcs = GixVcs::open(repo)?;
        let repo = vcs.workdir().to_path_buf();
        let name = name.unwrap_or_else(|| {
            repo.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default()
        });
        self.mutate(|s| {
            if s.workspaces.iter().any(|w| w.repo == repo) {
                bail!("{} is already a workspace", repo.display());
            }
            let ws = Workspace {
                id: short_id(),
                name,
                repo,
                templates: Vec::new(),
                network: s.settings.default_network.clone(),
                template_builds: BTreeMap::new(),
            };
            s.workspaces.push(ws.clone());
            Ok(ws)
        })
        .await
    }

    /// Remove a workspace from the harness. Its conversations must be deleted first.
    pub async fn remove_workspace(&self, id: &str) -> Result<()> {
        self.mutate(|s| {
            if s.conversations.iter().any(|c| c.workspace_id == id) {
                bail!("delete the workspace's conversations first");
            }
            s.workspaces.retain(|w| w.id != id);
            Ok(())
        })
        .await
    }

    /// Change a workspace's templates and network policy. Template conflicts are rejected here,
    /// not discovered later.
    pub async fn configure_workspace(
        &self,
        id: &str,
        templates: Vec<String>,
        network: NetworkPolicy,
    ) -> Result<Workspace> {
        let manifests = self.load_templates(&templates)?;
        let fake: Vec<_> = manifests
            .into_iter()
            .map(|m| TemplateMount {
                built: PathBuf::from("/"),
                manifest: m,
            })
            .collect();
        nucleus_templates::resolve(
            &fake,
            &BTreeMap::new(),
            &reserved_env(),
            self.backend.engine().supports_overlay(),
        )?;
        self.mutate(|s| {
            let ws = s.workspace_mut(id)?;
            ws.templates = templates;
            ws.network = network;
            Ok(ws.clone())
        })
        .await
    }

    fn load_templates(&self, names: &[String]) -> Result<Vec<TemplateManifest>> {
        names
            .iter()
            .map(|n| {
                let m = TemplateManifest::load(&self.templates.root().join(n))?;
                if &m.name != n {
                    bail!("template directory {n} declares name {}", m.name);
                }
                Ok(m)
            })
            .collect()
    }

    /// All approved templates in the library.
    pub fn available_templates(&self) -> Vec<TemplateManifest> {
        let mut out = Vec::new();
        if let Ok(rd) = std::fs::read_dir(self.templates.root()) {
            for e in rd.flatten() {
                if let Ok(m) = TemplateManifest::load(&e.path()) {
                    out.push(m);
                }
            }
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    pub async fn template_status(&self, workspace_id: &str) -> Result<Vec<TemplateStatus>> {
        let (ws, image) = {
            let s = self.state.lock().await;
            (s.workspace(workspace_id)?.clone(), s.settings.image.clone())
        };
        let builder = TemplateBuilder {
            backend: self.backend.as_ref(),
            root: self.paths.template_builds(),
        };
        let mut out = Vec::new();
        for name in &ws.templates {
            let status = async {
                let m = TemplateManifest::load(&self.templates.root().join(name))?;
                let image = m.build.image.clone().unwrap_or_else(|| image.clone());
                let image_id = self.backend.ensure_image(&image).await?;
                let id = nucleus_templates::identity(&m, &ws.repo, &image_id)?;
                Ok::<_, anyhow::Error>((m.description.clone(), builder.is_built(name, &id), id))
            }
            .await;
            out.push(match status {
                Ok((description, fresh, id)) => TemplateStatus {
                    name: name.clone(),
                    description,
                    fresh,
                    identity: Some(id),
                    error: None,
                },
                Err(e) => TemplateStatus {
                    name: name.clone(),
                    description: String::new(),
                    fresh: false,
                    identity: None,
                    error: Some(format!("{e:#}")),
                },
            });
        }
        Ok(out)
    }

    /// Build every template of the workspace that is missing or stale.
    pub async fn build_templates(&self, workspace_id: &str) -> Result<Vec<TemplateMount>> {
        let (ws, image) = {
            let s = self.state.lock().await;
            (s.workspace(workspace_id)?.clone(), s.settings.image.clone())
        };
        let builder = TemplateBuilder {
            backend: self.backend.as_ref(),
            root: self.paths.template_builds(),
        };
        let mut mounts = Vec::new();
        let mut identities = BTreeMap::new();
        for m in self.load_templates(&ws.templates)? {
            self.emit(HarnessEvent::Progress {
                message: format!("Preparing template {}", m.name),
            });
            let sink = self.sink.clone();
            let template = m.name.clone();
            let on_line = move |line: &str| {
                sink(HarnessEvent::BuildOutput {
                    template: template.clone(),
                    line: line.to_string(),
                })
            };
            let outcome = builder.build_streaming(&m, &ws.repo, &image, &on_line).await?;
            if outcome.built {
                tracing::info!(template = %m.name, identity = %outcome.identity, "template built");
                self.emit(HarnessEvent::Progress {
                    message: format!("Built template {} ({})", m.name, outcome.identity),
                });
            }
            identities.insert(m.name.clone(), outcome.identity.clone());
            mounts.push(TemplateMount {
                manifest: m,
                built: outcome.path,
            });
        }
        self.mutate(|s| {
            s.workspace_mut(workspace_id)?.template_builds = identities.clone();
            Ok(())
        })
        .await?;
        // Builds still recorded by another workspace stay; everything else for these templates goes.
        let keep: Vec<String> = self
            .state
            .lock()
            .await
            .workspaces
            .iter()
            .flat_map(|w| w.template_builds.values().cloned())
            .collect();
        for name in identities.keys() {
            builder.prune(name, &keep).await.ok();
        }
        Ok(mounts)
    }

    /// Output of the most recent build of a template, successful or not.
    pub fn template_build_log(&self, name: &str) -> Option<String> {
        TemplateBuilder {
            backend: self.backend.as_ref(),
            root: self.paths.template_builds(),
        }
        .last_log(name)
    }

    /// What the egress proxy allowed and denied for a conversation's sandbox, oldest first.
    pub async fn egress_log(&self, conversation_id: &str) -> Result<EgressLog> {
        let (conv, network) = {
            let s = self.state.lock().await;
            let conv = s.conversation(conversation_id)?.clone();
            let network = s.workspace(&conv.workspace_id)?.network.clone();
            (conv, network)
        };
        let required: Vec<String> = REQUIRED_HOSTS.iter().map(|h| h.to_string()).collect();
        let plan = nucleus_sandbox::EgressPlan::for_policy(&network, &required);
        let (mode, allowed) = match &plan {
            nucleus_sandbox::EgressPlan::Isolated => ("isolated", Vec::new()),
            nucleus_sandbox::EgressPlan::Proxied { allow } => ("proxied", allow.clone()),
            nucleus_sandbox::EgressPlan::Open => ("open", Vec::new()),
        };
        let entries = match plan {
            nucleus_sandbox::EgressPlan::Proxied { .. } => self.backend.egress_log(&conv.container, 1000).await?,
            _ => Vec::new(),
        };
        Ok(EgressLog {
            mode: mode.to_string(),
            allowed,
            entries,
        })
    }

    pub fn workspace_vcs(&self, ws: &Workspace) -> Result<GixVcs> {
        GixVcs::open_with_strategy(&ws.repo, self.strategy.clone())
    }

    async fn vcs_for(&self, workspace_id: &str) -> Result<GixVcs> {
        let ws = self.state.lock().await.workspace(workspace_id)?.clone();
        self.workspace_vcs(&ws)
    }

    pub async fn branches(&self, workspace_id: &str) -> Result<Vec<BranchInfo>> {
        self.vcs_for(workspace_id).await?.branches().await
    }

    /// Commits reachable from all branches, for the branch tree.
    pub async fn graph(&self, workspace_id: &str, limit: usize) -> Result<Vec<CommitInfo>> {
        let vcs = self.vcs_for(workspace_id).await?;
        let tips: Vec<String> = vcs.branches().await?.into_iter().map(|b| b.target).collect();
        vcs.graph(&tips, limit).await
    }

    /// Diff of two revisions, one file at a time.
    pub async fn diff_stream(&self, workspace_id: &str, from: &str, to: &str) -> Result<nucleus_vcs::DiffStream> {
        self.vcs_for(workspace_id).await?.diff_stream(from, to).await
    }

    /// The conversation's changes since its branch left the base, one file at a time.
    pub async fn conversation_diff_stream(&self, conversation_id: &str) -> Result<nucleus_vcs::DiffStream> {
        let conv = self.state.lock().await.conversation(conversation_id)?.clone();
        let vcs = self.vcs_for(&conv.workspace_id).await?;
        let base = vcs
            .merge_base(&conv.base_branch, &conv.branch)
            .await?
            .ok_or_else(|| anyhow!("{} and {} share no history", conv.base_branch, conv.branch))?;
        vcs.diff_stream(&base, &conv.branch).await
    }

    pub async fn diff(&self, workspace_id: &str, from: &str, to: &str) -> Result<Vec<FileDiff>> {
        self.vcs_for(workspace_id).await?.diff(from, to).await
    }

    // ---- conversations -----------------------------------------------------------------

    /// Start a conversation: branch from `base_branch`, add a worktree, start the container.
    pub async fn create_conversation(
        &self,
        workspace_id: &str,
        base_branch: &str,
        title: &str,
    ) -> Result<Conversation> {
        let ws = self.state.lock().await.workspace(workspace_id)?.clone();
        let vcs = self.workspace_vcs(&ws)?;
        let id = short_id();
        let branch = self.strategy.agent_branch(&id);
        let worktree = self.paths.worktrees().join(&id);
        let conv = Conversation {
            id: id.clone(),
            workspace_id: ws.id.clone(),
            title: if title.trim().is_empty() {
                "New conversation".into()
            } else {
                title.trim().into()
            },
            base_branch: base_branch.to_string(),
            branch: branch.clone(),
            worktree: worktree.clone(),
            container: format!("nucleus-{id}"),
            session_id: None,
            created: chrono::Utc::now().timestamp(),
            status: ConversationStatus::Idle,
            last_turn_skills: Vec::new(),
        };
        let setup = async {
            vcs.create_branch(&branch, base_branch).await?;
            vcs.add_worktree(&worktree, &branch).await?;
            // Persist before starting the container so a crash leaves nothing untracked.
            self.mutate(|s| {
                s.conversations.push(conv.clone());
                Ok(())
            })
            .await?;
            self.start_container(&conv).await
        };
        if let Err(e) = setup.await {
            tracing::warn!(conversation = %conv.id, "creating conversation failed: {e:#}");
            self.teardown(&conv, &vcs).await.ok();
            self.mutate(|s| {
                s.conversations.retain(|c| c.id != conv.id);
                Ok(())
            })
            .await
            .ok();
            return Err(e);
        }
        tracing::info!(conversation = %conv.id, branch = %conv.branch, base = %conv.base_branch, "conversation created");
        Ok(conv)
    }

    fn conversation_dirs(&self, id: &str) -> (PathBuf, PathBuf, PathBuf) {
        let base = self.paths.conversation(id);
        (base.join("home"), base.join("outbox"), base.join("support"))
    }

    /// Write the MCP server, tool index and MCP config the container reads.
    fn write_support(&self, conv_id: &str) -> Result<()> {
        let (_, _, support) = self.conversation_dirs(conv_id);
        std::fs::create_dir_all(&support)?;
        std::fs::write(support.join("mcp-server.js"), nucleus_tools::MCP_SERVER_JS)?;
        std::fs::write(
            support.join("gitfile"),
            format!("gitdir: {}\n", sandbox_git::GIT_DIR_MOUNT),
        )?;
        let tools = nucleus_tools::load_registry(self.tools.root());
        std::fs::write(
            support.join("tools.json"),
            serde_json::to_vec_pretty(&nucleus_tools::tools_index(&tools))?,
        )?;
        std::fs::write(
            support.join("mcp.json"),
            serde_json::to_vec_pretty(&nucleus_tools::mcp_config(SUPPORT_MOUNT))?,
        )?;
        Ok(())
    }

    async fn start_container(&self, conv: &Conversation) -> Result<()> {
        let (ws, settings) = {
            let s = self.state.lock().await;
            (s.workspace(&conv.workspace_id)?.clone(), s.settings.clone())
        };
        let templates = self.build_templates(&ws.id).await?;
        let image_env: BTreeMap<String, String> = self
            .backend
            .image_env(&settings.image)
            .await?
            .into_iter()
            .filter_map(|e| e.split_once('=').map(|(k, v)| (k.to_string(), v.to_string())))
            .collect();
        let resolved = nucleus_templates::resolve(
            &templates,
            &image_env,
            &reserved_env(),
            self.backend.engine().supports_overlay(),
        )?;
        let vcs = self.workspace_vcs(&ws)?;
        if !resolved.excludes.is_empty() {
            exclude::ensure_excluded(&vcs.common_dir(), &resolved.excludes)?;
        }

        let (home, outbox, support) = self.conversation_dirs(&conv.id);
        for d in [&home, &outbox, &support] {
            std::fs::create_dir_all(d)?;
        }
        self.write_support(&conv.id)?;

        let mut spec = ContainerSpec::new(&conv.container, &settings.image);
        spec.user = current_user();
        spec.workdir = Some(WORKSPACE_MOUNT.into());
        spec.network = ws.network.clone();
        spec.limits = settings.limits.to_sandbox();
        spec.required_hosts = REQUIRED_HOSTS.iter().map(|h| h.to_string()).collect();
        spec.labels.insert(CONVERSATION_LABEL.into(), conv.id.clone());
        spec.labels.insert("nucleus.workspace".into(), ws.id.clone());
        let rw = |source: &Path, target: &str| BindMount {
            source: source.into(),
            target: target.into(),
            mode: MountMode::ReadWrite,
        };
        let ro = |source: &Path, target: &str| BindMount {
            source: source.into(),
            target: target.into(),
            mode: MountMode::ReadOnly,
        };
        spec.binds.push(rw(&conv.worktree, WORKSPACE_MOUNT));
        spec.binds.push(rw(&home, HOME_MOUNT));
        // Approved skills, where the Claude CLI discovers SKILL.md files.
        spec.binds.push(ro(
            self.skills.library().root(),
            &format!("{HOME_MOUNT}/.claude/skills"),
        ));
        spec.binds.push(ro(self.tools.root(), nucleus_tools::TOOLS_MOUNT));
        spec.binds.push(ro(&support, SUPPORT_MOUNT));
        spec.binds.push(rw(&outbox, OUTBOX_MOUNT));
        // Git in the sandbox: a private git dir, the main repository's objects read-only, and a
        // pointer file over the worktree's `.git` (which names a host path).
        let git_dir = self.paths.conversation(&conv.id).join("git");
        std::fs::create_dir_all(&git_dir)?;
        spec.binds.push(rw(&git_dir, sandbox_git::GIT_DIR_MOUNT));
        spec.binds
            .push(ro(&vcs.common_dir().join("objects"), sandbox_git::MAIN_OBJECTS_MOUNT));
        spec.binds
            .push(ro(&support.join("gitfile"), &format!("{WORKSPACE_MOUNT}/.git")));
        spec.env
            .insert("NUCLEUS_GIT_DIR".into(), sandbox_git::GIT_DIR_MOUNT.into());
        spec.env
            .insert("NUCLEUS_MAIN_OBJECTS".into(), sandbox_git::MAIN_OBJECTS_MOUNT.into());
        spec.binds.extend(resolved.binds);
        let (volumes, cache_env) = caches::mounts(caches::DEFAULT_CACHES);
        spec.chown_paths = volumes.iter().map(|v| v.target.clone()).collect();
        spec.volumes = volumes;
        spec.env.extend(cache_env);
        spec.env.extend(resolved.env);
        spec.env.insert("HOME".into(), HOME_MOUNT.into());
        spec.env.insert("NUCLEUS_WORKSPACE".into(), WORKSPACE_MOUNT.into());
        spec.env.insert("NUCLEUS_OUTBOX_DIR".into(), OUTBOX_MOUNT.into());
        self.backend.remove(&conv.container).await.ok();
        self.backend.create(&spec).await?;
        Ok(())
    }

    /// Recreate the conversation's container if it is gone (after a reboot or engine restart).
    pub async fn ensure_container(&self, conversation_id: &str) -> Result<()> {
        let conv = self.state.lock().await.conversation(conversation_id)?.clone();
        let running = self
            .backend
            .list(Some((CONVERSATION_LABEL, &conv.id)))
            .await?
            .iter()
            .any(|c| c.name == conv.container && c.running);
        if !running {
            self.start_container(&conv).await?;
        } else {
            self.write_support(&conv.id)?;
        }
        Ok(())
    }

    /// Recreate the conversation's container, e.g. to apply new limits or network settings.
    /// The worktree and the CLI's home (sessions) survive.
    pub async fn restart_container(&self, conversation_id: &str) -> Result<()> {
        let lock = self.conversation_lock(conversation_id).await;
        let _guard = lock
            .try_lock()
            .map_err(|_| anyhow!("wait for the running turn to finish"))?;
        let conv = self.state.lock().await.conversation(conversation_id)?.clone();
        self.start_container(&conv).await?;
        tracing::info!(conversation = %conv.id, "sandbox restarted");
        Ok(())
    }

    fn system_append(&self, conv: &Conversation) -> String {
        format!(
            "You are running inside the nucleus harness, in a sandboxed container.\n\
             - /workspace is a git worktree on branch {branch}, created from {base}. You can use git normally on this branch \
             (commit as often as you like); the harness commits anything left uncommitted after every turn. \
             Do not switch branches or rewrite commits from earlier turns.\n\
             - Network access is restricted by the workspace policy; if a download is blocked, say which host you need.\n\
             - Dependencies from templates are mounted under /deps and are read-only.\n\
             - When you finish a task and learned something reusable, propose a skill with the nucleus propose_skill tool. \
             If you built a helper script worth keeping, put it with a test under {outbox}/tools/<name>/ and use propose_tool. \
             The user approves every proposal; only propose lessons that generalise.",
            branch = conv.branch,
            base = conv.base_branch,
            outbox = OUTBOX_MOUNT,
        )
    }

    /// Run one user turn. Streams events to the sink and returns once the turn is finished
    /// and its changes are committed.
    pub async fn send_message(&self, conversation_id: &str, prompt: &str) -> Result<TurnSummary> {
        let lock = self.conversation_lock(conversation_id).await;
        let _guard = lock
            .try_lock()
            .map_err(|_| anyhow!("this conversation is already running a turn"))?;
        self.ensure_container(conversation_id).await?;
        let (conv, settings) = {
            let s = self.state.lock().await;
            (s.conversation(conversation_id)?.clone(), s.settings.clone())
        };
        if !settings
            .provider_env
            .keys()
            .any(|k| k == "ANTHROPIC_API_KEY" || k == "CLAUDE_CODE_OAUTH_TOKEN")
        {
            bail!("set ANTHROPIC_API_KEY or CLAUDE_CODE_OAUTH_TOKEN in settings first");
        }
        let vcs = self.vcs_for(&conv.workspace_id).await?;
        let tip_before = vcs.resolve(&format!("refs/heads/{}", conv.branch)).await?;
        let sandbox_git = match self.sync_sandbox_git(&conv, &tip_before).await {
            Ok(enabled) => enabled,
            Err(e) => {
                tracing::warn!(conversation = %conv.id, "git in the sandbox is unavailable: {e:#}");
                false
            }
        };
        let launcher = Arc::new(SandboxLauncher {
            backend: self.backend.clone(),
            container: conv.container.clone(),
        });
        let config = ClaudeCliConfig {
            workdir: WORKSPACE_MOUNT.into(),
            model: settings.model.clone(),
            mcp_config: Some(format!("{SUPPORT_MOUNT}/mcp.json")),
            permission_mode: settings.permission_mode.clone(),
            env: settings.provider_env.clone(),
            ..Default::default()
        };
        let provider: Arc<dyn LlmProvider> = Arc::new(ClaudeCliProvider::new(launcher, config));
        self.running.lock().await.insert(conv.id.clone(), provider.clone());
        tracing::info!(conversation = %conv.id, "turn started");
        self.set_status(&conv.id, ConversationStatus::Running).await?;

        let mut agent = Agent::new(provider).with_session(conv.session_id.clone());
        agent.set_system_append(Some(self.system_append(&conv)));
        let sink = self.sink.clone();
        let conv_id = conv.id.clone();
        let mut used_skills = Vec::new();
        let result = agent
            .run_turn(prompt, |event| {
                if let AgentEvent::ToolUse { name, input, .. } = event
                    && name == "Skill"
                    && let Some(skill) = input
                        .get("skill")
                        .or_else(|| input.get("command"))
                        .and_then(|v| v.as_str())
                {
                    used_skills.push(skill.to_string());
                }
                sink(HarnessEvent::Agent {
                    conversation_id: conv_id.clone(),
                    event: event.clone(),
                });
            })
            .await;
        self.running.lock().await.remove(&conv.id);

        let summary = match result {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(conversation = %conv.id, "turn failed to start: {e:#}");
                self.set_status(&conv.id, ConversationStatus::Error).await?;
                return Err(e);
            }
        };
        tracing::info!(conversation = %conv.id, is_error = summary.is_error, exit_code = ?summary.exit_code, "turn finished");
        self.append_transcript(&conv.id, &summary.entries)?;
        used_skills.sort();
        used_skills.dedup();
        for skill in &used_skills {
            self.skills.usage().record_use(skill).ok();
            let outcome = if summary.is_error {
                Outcome::Failure
            } else {
                Outcome::Success
            };
            self.skills.usage().record_outcome(skill, outcome).ok();
        }
        if sandbox_git {
            match self.import_sandbox_commits(&conv, vcs.workdir(), &tip_before).await {
                Ok(sandbox_git::Import::Imported(commit)) => {
                    tracing::info!(conversation = %conv.id, %commit, "imported the agent's own commits");
                    self.emit(HarnessEvent::Committed {
                        conversation_id: conv.id.clone(),
                        commit,
                    });
                }
                Ok(sandbox_git::Import::Rejected(commit)) => {
                    tracing::warn!(conversation = %conv.id, %commit, "agent rewrote history; commits not imported");
                    self.emit(HarnessEvent::Agent {
                        conversation_id: conv.id.clone(),
                        event: AgentEvent::Error {
                            message: "The agent rewrote commits that were already saved; its history was not imported, but its changes are committed.".into(),
                        },
                    });
                }
                Ok(_) => {}
                Err(e) => tracing::warn!(conversation = %conv.id, "importing sandbox commits failed: {e:#}"),
            }
        }
        let message = format!("Agent turn: {}\n\nConversation: {}", first_line(prompt, 60), conv.id);
        // During an update from the base, committing concludes the merge: only once every
        // conflict marker is gone.
        let unresolved = match vcs.merge_conflicts(&conv.worktree).await {
            Ok(Some(paths)) => self.unresolved(&conv.worktree, &paths),
            _ => Vec::new(),
        };
        let commit_result = if unresolved.is_empty() {
            vcs.commit_all(&conv.worktree, &message).await
        } else {
            self.emit(HarnessEvent::Agent {
                conversation_id: conv.id.clone(),
                event: AgentEvent::Error {
                    message: format!("Conflict markers remain in {}; nothing was committed. Ask the agent again or abort the update.", unresolved.join(", ")),
                },
            });
            Ok(None)
        };
        match commit_result {
            Ok(Some(commit)) => {
                tracing::info!(conversation = %conv.id, %commit, "committed agent changes");
                self.emit(HarnessEvent::Committed {
                    conversation_id: conv.id.clone(),
                    commit,
                })
            }
            Ok(None) => {}
            Err(e) => {
                tracing::error!(conversation = %conv.id, "committing agent changes failed: {e:#}");
                self.emit(HarnessEvent::Agent {
                    conversation_id: conv.id.clone(),
                    event: AgentEvent::Error {
                        message: format!("committing agent changes failed: {e:#}"),
                    },
                })
            }
        }
        if sandbox_git {
            let tip = vcs.resolve(&format!("refs/heads/{}", conv.branch)).await?;
            if let Err(e) = self.sync_sandbox_git(&conv, &tip).await {
                tracing::warn!(conversation = %conv.id, "syncing git in the sandbox failed: {e:#}");
            }
        }
        self.process_outbox(&conv).await;
        let session = summary.session_id.clone();
        let status = if summary.is_error {
            ConversationStatus::Error
        } else {
            ConversationStatus::Idle
        };
        self.mutate(|s| {
            let c = s.conversation_mut(&conv.id)?;
            c.session_id = session;
            c.status = status;
            c.last_turn_skills = used_skills;
            Ok(())
        })
        .await?;
        self.emit(HarnessEvent::Status {
            conversation_id: conv.id.clone(),
            status,
        });
        Ok(summary)
    }

    async fn set_status(&self, id: &str, status: ConversationStatus) -> Result<()> {
        self.mutate(|s| {
            s.conversation_mut(id)?.status = status;
            Ok(())
        })
        .await?;
        self.emit(HarnessEvent::Status {
            conversation_id: id.to_string(),
            status,
        });
        Ok(())
    }

    pub async fn cancel(&self, conversation_id: &str) -> Result<()> {
        let provider = self.running.lock().await.get(conversation_id).cloned();
        match provider {
            Some(p) => p.cancel().await,
            None => Ok(()),
        }
    }

    /// The user says the last turn's result was wrong: the skills it used get a negative
    /// outcome instead of the success recorded automatically. Each turn can be rated once.
    /// Returns the affected skills.
    pub async fn mark_last_turn_wrong(&self, conversation_id: &str) -> Result<Vec<String>> {
        let skills = self
            .mutate(|s| {
                Ok(std::mem::take(
                    &mut s.conversation_mut(conversation_id)?.last_turn_skills,
                ))
            })
            .await?;
        for skill in &skills {
            self.skills.usage().mark_wrong(skill)?;
        }
        if !skills.is_empty() {
            tracing::info!(conversation = %conversation_id, ?skills, "last turn marked as wrong");
        }
        Ok(skills)
    }

    fn append_transcript(&self, id: &str, entries: &[TranscriptEntry]) -> Result<()> {
        use std::io::Write;
        let path = self.paths.conversation(id).join("transcript.jsonl");
        std::fs::create_dir_all(path.parent().unwrap())?;
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path)?;
        for e in entries {
            writeln!(f, "{}", serde_json::to_string(e)?)?;
        }
        Ok(())
    }

    pub fn transcript(&self, id: &str) -> Result<Vec<TranscriptEntry>> {
        let path = self.paths.conversation(id).join("transcript.jsonl");
        let Ok(text) = std::fs::read_to_string(path) else {
            return Ok(Vec::new());
        };
        Ok(text.lines().filter_map(|l| serde_json::from_str(l).ok()).collect())
    }

    /// Changes the conversation made relative to where its branch diverged from the base.
    pub async fn conversation_diff(&self, conversation_id: &str) -> Result<Vec<FileDiff>> {
        let conv = self.state.lock().await.conversation(conversation_id)?.clone();
        let vcs = self.vcs_for(&conv.workspace_id).await?;
        let base = vcs
            .merge_base(&conv.base_branch, &conv.branch)
            .await?
            .ok_or_else(|| anyhow!("{} and {} share no history", conv.base_branch, conv.branch))?;
        vcs.diff(&base, &conv.branch).await
    }

    /// Commits on the agent branch not reachable from any local or origin branch.
    pub async fn unmerged_commits(&self, conversation_id: &str) -> Result<Vec<CommitInfo>> {
        let conv = self.state.lock().await.conversation(conversation_id)?.clone();
        let vcs = self.vcs_for(&conv.workspace_id).await?;
        let hidden: Vec<String> = vcs
            .branches()
            .await?
            .into_iter()
            .filter(|b| b.kind != nucleus_vcs::BranchKind::Agent)
            .map(|b| b.target)
            .collect();
        vcs.unique_commits(&conv.branch, &hidden).await
    }

    pub async fn merge_conversation(&self, conversation_id: &str, into: &str) -> Result<MergeOutcome> {
        let lock = self.conversation_lock(conversation_id).await;
        let _guard = lock
            .try_lock()
            .map_err(|_| anyhow!("wait for the running turn to finish"))?;
        let conv = self.state.lock().await.conversation(conversation_id)?.clone();
        if self.strategy.conversation_of(into).is_some() {
            bail!("merge into a local branch, not another agent branch");
        }
        let vcs = self.vcs_for(&conv.workspace_id).await?;
        vcs.merge(
            into,
            &format!("refs/heads/{}", conv.branch),
            &format!("Merge {}: {}", conv.branch, conv.title),
        )
        .await
    }

    pub async fn rename_conversation(&self, id: &str, title: &str) -> Result<()> {
        self.mutate(|s| {
            s.conversation_mut(id)?.title = title.trim().to_string();
            Ok(())
        })
        .await
    }

    /// Delete a conversation together with its branch, worktree and container.
    pub async fn delete_conversation(&self, conversation_id: &str, mode: DeleteMode) -> Result<DeleteOutcome> {
        let lock = self.conversation_lock(conversation_id).await;
        let _guard = lock
            .try_lock()
            .map_err(|_| anyhow!("wait for the running turn to finish or cancel it"))?;
        let conv = self.state.lock().await.conversation(conversation_id)?.clone();
        let vcs = self.vcs_for(&conv.workspace_id).await?;
        // An unfinished update from the base would otherwise be committed with its markers.
        if vcs.merge_conflicts(&conv.worktree).await?.is_some() {
            vcs.abort_merge(&conv.worktree).await?;
        }
        // Commit anything left in the worktree so the unmerged check sees it.
        vcs.commit_all(
            &conv.worktree,
            &format!("Uncommitted agent changes\n\nConversation: {}", conv.id),
        )
        .await
        .ok();
        match &mode {
            DeleteMode::Check => {
                let unmerged = self.unmerged_commits(conversation_id).await?;
                if !unmerged.is_empty() {
                    return Ok(DeleteOutcome::NeedsConfirmation { unmerged });
                }
            }
            DeleteMode::Discard => {}
            DeleteMode::MergeInto { branch } => {
                drop(_guard);
                let outcome = self.merge_conversation(conversation_id, branch).await?;
                if let MergeOutcome::Conflicts { paths } = outcome {
                    return Ok(DeleteOutcome::MergeConflicts { paths });
                }
                return Box::pin(self.delete_conversation(conversation_id, DeleteMode::Discard)).await;
            }
            DeleteMode::KeepCopy { branch } => {
                if self.strategy.conversation_of(branch).is_some() {
                    bail!("keep the copy outside the agent/ namespace");
                }
                vcs.create_branch(branch, &format!("refs/heads/{}", conv.branch))
                    .await?;
            }
        }
        self.cancel(conversation_id).await.ok();
        self.teardown(&conv, &vcs).await?;
        self.mutate(|s| {
            s.conversations.retain(|c| c.id != conv.id);
            Ok(())
        })
        .await?;
        self.locks.lock().await.remove(conversation_id);
        tracing::info!(conversation = %conv.id, ?mode, "conversation deleted");
        Ok(DeleteOutcome::Deleted)
    }

    async fn teardown(&self, conv: &Conversation, vcs: &GixVcs) -> Result<()> {
        self.backend.remove(&conv.container).await?;
        vcs.remove_worktree(&conv.worktree).await?;
        if vcs.branches().await?.iter().any(|b| b.name == conv.branch) {
            vcs.delete_branch(&conv.branch).await?;
        }
        let dir = self.paths.conversation(&conv.id);
        if dir.exists() {
            tokio::fs::remove_dir_all(dir).await?;
        }
        Ok(())
    }

    /// Remove agent branches, worktrees, containers and conversation directories whose
    /// conversation no longer exists.
    pub async fn cleanup_orphans(&self) -> Result<CleanupReport> {
        let state = self.snapshot().await;
        let known: std::collections::HashSet<&str> = state.conversations.iter().map(|c| c.id.as_str()).collect();
        let mut report = CleanupReport::default();
        let mut errors = Vec::new();
        for c in self.backend.list(None).await? {
            if let Some(id) = c.labels.get(CONVERSATION_LABEL)
                && !known.contains(id.as_str())
            {
                match self.backend.remove(&c.name).await {
                    Ok(()) => report.containers.push(c.name),
                    Err(e) => errors.push(format!("container {}: {e:#}", c.name)),
                }
            }
        }
        let worktrees_root = canonical(&self.paths.worktrees());
        for ws in &state.workspaces {
            let vcs = match self.workspace_vcs(ws) {
                Ok(v) => v,
                Err(e) => {
                    errors.push(format!("workspace {}: {e:#}", ws.name));
                    continue;
                }
            };
            for wt in vcs.worktrees().await.unwrap_or_default() {
                // Compare canonical paths: temp and home directories are often symlinks
                // (/var -> /private/var on macOS).
                let ours = canonical(&wt.path).starts_with(&worktrees_root);
                let conversation = wt.branch.as_deref().and_then(|b| self.strategy.conversation_of(b));
                if ours && conversation.is_none_or(|id| !known.contains(id.as_str())) {
                    match vcs.remove_worktree(&wt.path).await {
                        Ok(()) => report.worktrees.push(wt.path),
                        Err(e) => errors.push(format!("worktree {}: {e:#}", wt.path.display())),
                    }
                }
            }
            for b in vcs.branches().await.unwrap_or_default() {
                if let Some(id) = self.strategy.conversation_of(&b.name)
                    && b.kind == nucleus_vcs::BranchKind::Agent
                    && !known.contains(id.as_str())
                {
                    match vcs.delete_branch(&b.name).await {
                        Ok(()) => report.branches.push(b.name),
                        Err(e) => errors.push(format!("branch {}: {e:#}", b.name)),
                    }
                }
            }
        }
        for dir in [self.paths.root.join("conversations"), self.paths.worktrees()] {
            if let Ok(rd) = std::fs::read_dir(&dir) {
                for e in rd.flatten() {
                    if !known.contains(e.file_name().to_string_lossy().as_ref()) {
                        match std::fs::remove_dir_all(e.path()) {
                            Ok(()) if dir == self.paths.worktrees() => report.worktrees.push(e.path()),
                            Ok(()) => {}
                            Err(err) => errors.push(format!("{}: {err}", e.path().display())),
                        }
                    }
                }
            }
        }
        for e in &errors {
            tracing::warn!("cleanup: {e}");
        }
        report.errors = errors;
        report.worktrees.sort();
        report.worktrees.dedup();
        Ok(report)
    }
}

/// Environment variables owned by the harness (cache locations and the like). A template
/// setting one of them is a conflict.
fn reserved_env() -> BTreeMap<String, String> {
    let (_, env) = caches::mounts(caches::DEFAULT_CACHES);
    let mut owners: BTreeMap<String, String> = env
        .into_keys()
        .map(|k| (k, "the shared package caches".to_string()))
        .collect();
    for k in [
        "HOME",
        "NUCLEUS_WORKSPACE",
        "NUCLEUS_OUTBOX_DIR",
        "HTTPS_PROXY",
        "HTTP_PROXY",
    ] {
        owners.insert(k.into(), "the harness".into());
    }
    owners
}

/// Canonical form of a path for comparisons; the path itself when it does not exist.
fn canonical(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

fn short_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..12].to_string()
}

fn first_line(s: &str, max: usize) -> String {
    let line = s.lines().next().unwrap_or_default().trim();
    if line.chars().count() > max {
        format!("{}…", line.chars().take(max).collect::<String>())
    } else {
        line.to_string()
    }
}
