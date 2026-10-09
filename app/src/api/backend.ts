import type {
  AppInfo,
  AppState,
  BranchInfo,
  CleanupReport,
  CommitInfo,
  EgressLog,
  Conversation,
  DeleteMode,
  DeleteOutcome,
  FileDiff,
  HarnessEvent,
  LogEntry,
  LogLevel,
  LibraryKind,
  MergeOutcome,
  NetworkPolicy,
  Proposal,
  ProposalDetail,
  RebaseOutcome,
  Settings,
  SkillSummary,
  TemplateManifest,
  TemplateStatus,
  ToolManifest,
  TranscriptEntry,
  TurnSummary,
  Workspace,
} from "./types";

/**
 * Everything the UI can ask of the harness. Implemented over Tauri IPC in the app and by an
 * in-memory mock in the browser, unit tests and end-to-end tests.
 */
export interface Backend {
  /** Connect to the container engine and open the data directory. Safe to call again to retry. */
  init(): Promise<AppInfo>;
  state(): Promise<AppState>;
  updateSettings(settings: Settings): Promise<void>;
  /** Folder picker; `null` when unavailable or cancelled. */
  pickDirectory(): Promise<string | null>;

  addWorkspace(path: string, name: string | null): Promise<Workspace>;
  removeWorkspace(id: string): Promise<void>;
  configureWorkspace(id: string, templates: string[], network: NetworkPolicy): Promise<Workspace>;
  availableTemplates(): Promise<TemplateManifest[]>;
  templateStatus(workspaceId: string): Promise<TemplateStatus[]>;
  buildTemplates(workspaceId: string): Promise<void>;
  /** Output of the most recent build of a template, or null if it was never built. */
  templateBuildLog(name: string): Promise<string | null>;

  branches(workspaceId: string): Promise<BranchInfo[]>;
  remotes(workspaceId: string): Promise<string[]>;
  /** Fetch one remote, or all when `remote` is null. */
  fetch(workspaceId: string, remote: string | null): Promise<void>;
  /** Push a local (non-agent) branch; returns git's report. */
  push(workspaceId: string, branch: string, remote: string): Promise<string>;
  graph(workspaceId: string, limit: number): Promise<CommitInfo[]>;
  diff(workspaceId: string, from: string, to: string): Promise<FileDiff[]>;
  /** Like `diff`, delivering files as they are computed; resolves to the file count. */
  diffStream(workspaceId: string, from: string, to: string, onFile: (f: FileDiff) => void): Promise<number>;

  createConversation(workspaceId: string, baseBranch: string, title: string): Promise<Conversation>;
  renameConversation(id: string, title: string): Promise<void>;
  deleteConversation(id: string, mode: DeleteMode): Promise<DeleteOutcome>;
  sendMessage(id: string, prompt: string): Promise<TurnSummary>;
  cancel(id: string): Promise<void>;
  /** Recreate a conversation's sandbox to apply new settings. */
  restartSandbox(id: string): Promise<void>;
  /** What the egress proxy allowed and denied for a conversation's sandbox. */
  egressLog(id: string): Promise<EgressLog>;
  transcript(id: string): Promise<TranscriptEntry[]>;
  conversationDiff(id: string): Promise<FileDiff[]>;
  conversationDiffStream(id: string, onFile: (f: FileDiff) => void): Promise<number>;
  unmergedCommits(id: string): Promise<CommitInfo[]>;
  mergeConversation(id: string, into: string): Promise<MergeOutcome>;
  /** Merge the base branch into the agent branch; conflicts stay for the agent to resolve. */
  updateFromBase(id: string): Promise<MergeOutcome>;
  rebaseConversation(id: string): Promise<RebaseOutcome>;
  /** Conflicted paths while an update from the base is in progress, else null. */
  mergeState(id: string): Promise<string[] | null>;
  abortUpdate(id: string): Promise<void>;
  /** Run a turn asking the agent to resolve the conflicts; the merge is concluded after it. */
  resolveConflicts(id: string): Promise<TurnSummary>;

  proposals(): Promise<Proposal[]>;
  proposal(kind: LibraryKind, id: string): Promise<ProposalDetail>;
  approve(kind: LibraryKind, id: string): Promise<string>;
  reject(kind: LibraryKind, id: string): Promise<void>;
  history(kind: LibraryKind, limit: number): Promise<CommitInfo[]>;
  revert(kind: LibraryKind, commit: string): Promise<string>;
  skills(): Promise<SkillSummary[]>;
  tools(): Promise<ToolManifest[]>;
  skillSource(name: string): Promise<string>;
  templateSource(name: string): Promise<string>;
  /** User-authored proposals; they go through review like the agent's. */
  proposeSkill(content: string, rationale: string): Promise<Proposal>;
  proposeSkillRemoval(name: string, rationale: string): Promise<Proposal>;
  proposeTemplate(manifest: string, rationale: string): Promise<Proposal>;
  /** Record that the last turn's result was wrong; returns the skills it penalised. */
  markLastTurnWrong(id: string): Promise<string[]>;

  cleanupOrphans(): Promise<CleanupReport>;
  buildAgentImage(): Promise<string>;

  /** Recent log entries at or above `minLevel`, oldest first. */
  recentLogs(minLevel: LogLevel | null): Promise<LogEntry[]>;

  /** Subscribe to streamed harness events. Returns an unsubscribe function. */
  subscribe(listener: (event: HarnessEvent) => void): () => void;
}

/** Placeholder the backend returns instead of secret values. */
export const SECRET_MASK = "••••••••";
