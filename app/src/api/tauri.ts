import { Channel, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { Backend } from "./backend";
import type { FileDiff, HarnessEvent } from "./types";

function channel(onFile: (f: FileDiff) => void): Channel<FileDiff> {
  const ch = new Channel<FileDiff>();
  ch.onmessage = onFile;
  return ch;
}

/** Event name the Rust side emits harness events on. */
export const EVENT_NAME = "nucleus://event";

/** Backend over Tauri IPC. Command names match `app/src-tauri/src/commands.rs`. */
export function tauriBackend(): Backend {
  const call = <T>(cmd: string, args?: Record<string, unknown>) => invoke<T>(cmd, args);
  return {
    init: () => call("init"),
    state: () => call("get_state"),
    updateSettings: (settings) => call("update_settings", { settings }),
    pickDirectory: () => call("pick_directory"),
    addWorkspace: (path, name) => call("add_workspace", { path, name }),
    removeWorkspace: (id) => call("remove_workspace", { id }),
    configureWorkspace: (id, templates, network) => call("configure_workspace", { id, templates, network }),
    availableTemplates: () => call("available_templates"),
    templateStatus: (workspaceId) => call("template_status", { workspaceId }),
    buildTemplates: (workspaceId) => call("build_templates", { workspaceId }),
    templateBuildLog: (name) => call("template_build_log", { name }),
    branches: (workspaceId) => call("branches", { workspaceId }),
    remotes: (workspaceId) => call("remotes", { workspaceId }),
    fetch: (workspaceId, remote) => call("fetch", { workspaceId, remote }),
    push: (workspaceId, branch, remote) => call("push", { workspaceId, branch, remote }),
    graph: (workspaceId, limit) => call("graph", { workspaceId, limit }),
    diff: (workspaceId, from, to) => call("diff", { workspaceId, from, to }),
    diffStream: (workspaceId, from, to, onFile) => call("diff_stream", { workspaceId, from, to, onFile: channel(onFile) }),
    createConversation: (workspaceId, baseBranch, title) => call("create_conversation", { workspaceId, baseBranch, title }),
    renameConversation: (id, title) => call("rename_conversation", { id, title }),
    deleteConversation: (id, mode) => call("delete_conversation", { id, mode }),
    sendMessage: (id, prompt) => call("send_message", { id, prompt }),
    cancel: (id) => call("cancel", { id }),
    restartSandbox: (id) => call("restart_sandbox", { id }),
    egressLog: (id) => call("egress_log", { id }),
    transcript: (id) => call("transcript", { id }),
    conversationDiff: (id) => call("conversation_diff", { id }),
    conversationDiffStream: (id, onFile) => call("conversation_diff_stream", { id, onFile: channel(onFile) }),
    unmergedCommits: (id) => call("unmerged_commits", { id }),
    mergeConversation: (id, into) => call("merge_conversation", { id, into }),
    updateFromBase: (id) => call("update_from_base", { id }),
    rebaseConversation: (id) => call("rebase_conversation", { id }),
    mergeState: (id) => call("merge_state", { id }),
    abortUpdate: (id) => call("abort_update", { id }),
    resolveConflicts: (id) => call("resolve_conflicts", { id }),
    proposals: () => call("proposals"),
    proposal: (kind, id) => call("proposal", { kind, id }),
    approve: (kind, id) => call("approve", { kind, id }),
    reject: (kind, id) => call("reject", { kind, id }),
    history: (kind, limit) => call("history", { kind, limit }),
    revert: (kind, commit) => call("revert", { kind, commit }),
    skills: () => call("skills"),
    tools: () => call("tools"),
    skillSource: (name) => call("skill_source", { name }),
    templateSource: (name) => call("template_source", { name }),
    proposeSkill: (content, rationale) => call("propose_skill", { content, rationale }),
    proposeSkillRemoval: (name, rationale) => call("propose_skill_removal", { name, rationale }),
    proposeTemplate: (manifest, rationale) => call("propose_template", { manifest, rationale }),
    markLastTurnWrong: (id) => call("mark_last_turn_wrong", { id }),
    cleanupOrphans: () => call("cleanup_orphans"),
    buildAgentImage: () => call("build_agent_image"),
    recentLogs: (minLevel) => call("recent_logs", { minLevel }),
    subscribe(listener) {
      let unlisten: (() => void) | null = null;
      let cancelled = false;
      listen<HarnessEvent>(EVENT_NAME, (e) => listener(e.payload)).then((u) => {
        if (cancelled) u();
        else unlisten = u;
      });
      return () => {
        cancelled = true;
        unlisten?.();
      };
    },
  };
}

export function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}
