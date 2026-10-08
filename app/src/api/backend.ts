import type {
  AppInfo,
  AppState,
  BranchInfo,
  CleanupReport,
  CommitInfo,
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
  Settings,
  SkillSummary,
  TemplateManifest,
  TemplateStatus,
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

  branches(workspaceId: string): Promise<BranchInfo[]>;
  graph(workspaceId: string, limit: number): Promise<CommitInfo[]>;
  diff(workspaceId: string, from: string, to: string): Promise<FileDiff[]>;

  createConversation(workspaceId: string, baseBranch: string, title: string): Promise<Conversation>;
  renameConversation(id: string, title: string): Promise<void>;
  deleteConversation(id: string, mode: DeleteMode): Promise<DeleteOutcome>;
  sendMessage(id: string, prompt: string): Promise<TurnSummary>;
  cancel(id: string): Promise<void>;
  /** Recreate a conversation's sandbox to apply new settings. */
  restartSandbox(id: string): Promise<void>;
  transcript(id: string): Promise<TranscriptEntry[]>;
  conversationDiff(id: string): Promise<FileDiff[]>;
  unmergedCommits(id: string): Promise<CommitInfo[]>;
  mergeConversation(id: string, into: string): Promise<MergeOutcome>;

  proposals(): Promise<Proposal[]>;
  proposal(kind: LibraryKind, id: string): Promise<ProposalDetail>;
  approve(kind: LibraryKind, id: string): Promise<string>;
  reject(kind: LibraryKind, id: string): Promise<void>;
  history(kind: LibraryKind, limit: number): Promise<CommitInfo[]>;
  revert(kind: LibraryKind, commit: string): Promise<string>;
  skills(): Promise<SkillSummary[]>;

  cleanupOrphans(): Promise<CleanupReport>;
  buildAgentImage(): Promise<string>;

  /** Recent log entries at or above `minLevel`, oldest first. */
  recentLogs(minLevel: LogLevel | null): Promise<LogEntry[]>;

  /** Subscribe to streamed harness events. Returns an unsubscribe function. */
  subscribe(listener: (event: HarnessEvent) => void): () => void;
}

/** Placeholder the backend returns instead of secret values. */
export const SECRET_MASK = "••••••••";
