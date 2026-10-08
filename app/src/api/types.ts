// Mirrors of the Rust types serialised over IPC. Keep in sync with crates/*/src (serde shapes).

export type BranchKind = "origin" | "local" | "agent";
export type ConversationStatus = "idle" | "running" | "error";
export type LibraryKind = "skills" | "tools" | "templates";
export type Engine = "podman" | "docker";

export type NetworkPolicy = { mode: "none" } | { mode: "allowlist"; hosts: string[] } | { mode: "full" };

export interface Settings {
  image: string;
  model: string | null;
  /** Values of secrets come back masked; send the mask to keep the stored value. */
  provider_env: Record<string, string>;
  default_network: NetworkPolicy;
  /** Claude CLI `--permission-mode`. */
  permission_mode: PermissionMode;
  limits: ResourceLimits;
}

export const PERMISSION_MODES = ["bypassPermissions", "acceptEdits", "auto", "dontAsk", "manual", "plan"] as const;
export type PermissionMode = (typeof PERMISSION_MODES)[number];

export interface ResourceLimits {
  memory_mb: number | null;
  cpus: number | null;
  pids: number | null;
}

export interface Workspace {
  id: string;
  name: string;
  repo: string;
  templates: string[];
  network: NetworkPolicy;
  template_builds: Record<string, string>;
}

export interface Conversation {
  id: string;
  workspace_id: string;
  title: string;
  base_branch: string;
  branch: string;
  worktree: string;
  container: string;
  session_id: string | null;
  created: number;
  status: ConversationStatus;
}

export interface BranchInfo {
  name: string;
  full_ref: string;
  kind: BranchKind;
  target: string;
  is_head: boolean;
}

export interface CommitInfo {
  id: string;
  summary: string;
  message: string;
  author_name: string;
  author_email: string;
  time: number;
  parents: string[];
}

export type FileStatus = "added" | "deleted" | "modified" | "renamed" | "copied";

export interface FileDiff {
  path: string;
  old_path: string | null;
  status: FileStatus;
  binary: boolean;
  additions: number;
  deletions: number;
  patch: string;
}

export type AgentEvent =
  | { type: "session_started"; session_id: string; model: string | null; tools: string[] }
  | { type: "text_delta"; text: string }
  | { type: "thinking_delta"; text: string }
  | { type: "assistant_text"; text: string }
  | { type: "tool_use"; id: string; name: string; input: unknown }
  | { type: "tool_result"; tool_use_id: string; content: string; is_error: boolean }
  | {
      type: "turn_completed";
      is_error: boolean;
      result: string | null;
      session_id: string | null;
      cost_usd: number | null;
      duration_ms: number | null;
      num_turns: number | null;
    }
  | { type: "stderr"; text: string }
  | { type: "error"; message: string }
  | { type: "process_exited"; code: number | null };

export interface Proposal {
  id: string;
  kind: LibraryKind;
  title: string;
  rationale: string;
  source: string | null;
  commit: string;
  created: number;
}

export interface ProposalDetail {
  proposal: Proposal;
  diff: FileDiff[];
}

export type HarnessEvent =
  | { type: "agent"; conversation_id: string; event: AgentEvent }
  | { type: "status"; conversation_id: string; status: ConversationStatus }
  | { type: "committed"; conversation_id: string; commit: string }
  | { type: "proposal_created"; proposal: Proposal }
  | { type: "proposal_failed"; conversation_id: string; kind: LibraryKind; error: string }
  | { type: "progress"; message: string };

export type TranscriptEntry =
  | { role: "user"; text: string }
  | { role: "assistant"; text: string }
  | { role: "tool_use"; id: string; name: string; input: unknown }
  | { role: "tool_result"; tool_use_id: string; content: string; is_error: boolean }
  | { role: "error"; message: string };

export interface TurnSummary {
  session_id: string | null;
  is_error: boolean;
  cost_usd: number | null;
  exit_code: number | null;
  entries: TranscriptEntry[];
}

export type DeleteMode =
  | { mode: "check" }
  | { mode: "discard" }
  | { mode: "merge_into"; branch: string }
  | { mode: "keep_copy"; branch: string };

export type DeleteOutcome =
  | { result: "deleted" }
  | { result: "needs_confirmation"; unmerged: CommitInfo[] }
  | { result: "merge_conflicts"; paths: string[] };

export type MergeOutcome =
  | { kind: "up_to_date" }
  | { kind: "fast_forward"; commit: string }
  | { kind: "merged"; commit: string }
  | { kind: "conflicts"; paths: string[] };

export type MountKind = { mode: "readonly" } | { mode: "overlay" } | { mode: "worktree"; path: string };

export interface TemplateManifest {
  name: string;
  description: string;
  mount: MountKind;
  env: Record<string, string>;
  path_env: Record<string, string[]>;
  build: { image: string | null; lockfiles: string[]; command: string; workdir: string | null; network: string[] };
}

export interface TemplateStatus {
  name: string;
  description: string;
  fresh: boolean;
  identity: string | null;
  error: string | null;
}

export interface SkillStats {
  uses: number;
  successes: number;
  failures: number;
  negative: number;
  last_used: number | null;
}

export interface SkillSummary {
  name: string;
  description: string;
  when_to_use: string | null;
  stats: SkillStats;
}

export interface CleanupReport {
  containers: string[];
  worktrees: string[];
  branches: string[];
}

/** Startup status. `error` is set when the container engine or data directory is unusable. */
export interface AppInfo {
  ready: boolean;
  engine: Engine | null;
  data_dir: string;
  error: string | null;
}

export interface AppState {
  settings: Settings;
  workspaces: Workspace[];
  conversations: Conversation[];
}

export type LogLevel = "error" | "warn" | "info" | "debug" | "trace";

export interface LogEntry {
  /** Milliseconds since the unix epoch. */
  time: number;
  level: LogLevel;
  target: string;
  message: string;
}
