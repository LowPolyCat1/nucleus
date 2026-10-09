import { createContext, createStore, useContext } from "solid-js";
import type { Backend } from "./api/backend";
import type {
  AppInfo,
  Conversation,
  DeleteMode,
  DeleteOutcome,
  HarnessEvent,
  LibraryKind,
  MergeOutcome,
  NetworkPolicy,
  Proposal,
  Settings,
  Workspace,
} from "./api/types";
import { applyEvent, emptyChat, fromTranscript, startTurn, type ChatState } from "./lib/chat";
import { errorMessage } from "./lib/format";

export type View = "workspace" | "conversation" | "proposals" | "skills" | "settings" | "logs";
export type ConversationTab = "chat" | "changes" | "network";

export interface Toast {
  id: number;
  kind: "info" | "success" | "error";
  text: string;
}

export interface UiState {
  info: AppInfo | null;
  loading: boolean;
  settings: Settings | null;
  workspaces: Workspace[];
  conversations: Conversation[];
  selectedWorkspace: string | null;
  selectedConversation: string | null;
  view: View;
  tab: ConversationTab;
  chats: Record<string, ChatState>;
  proposals: Proposal[];
  toasts: Toast[];
  progress: string | null;
  /** Bumped whenever repository contents may have changed, so views refetch. */
  repoVersion: number;
  /** Live output of template builds, per template, while the app runs. */
  buildOutput: Record<string, string[]>;
  libraryVersion: number;
}

export function createApp(backend: Backend, options: { toastMs?: number } = {}) {
  const toastMs = options.toastMs ?? 6000;
  const [state, setState] = createStore<UiState>({
    info: null,
    loading: true,
    settings: null,
    workspaces: [],
    conversations: [],
    selectedWorkspace: null,
    selectedConversation: null,
    view: "workspace",
    tab: "chat",
    chats: {},
    proposals: [],
    toasts: [],
    progress: null,
    repoVersion: 0,
    buildOutput: {},
    libraryVersion: 0,
  });
  let toastId = 0;

  const toast = (kind: Toast["kind"], text: string) => {
    const id = ++toastId;
    setState((s) => {
      s.toasts.push({ id, kind, text });
    });
    if (toastMs > 0) setTimeout(() => dismissToast(id), toastMs);
    return id;
  };
  const dismissToast = (id: number) =>
    setState((s) => {
      const i = s.toasts.findIndex((t) => t.id === id);
      if (i >= 0) s.toasts.splice(i, 1);
    });

  /** Run an action, turning failures into an error toast. Returns undefined on failure. */
  async function attempt<T>(f: () => Promise<T>, success?: string): Promise<T | undefined> {
    try {
      const r = await f();
      if (success) toast("success", success);
      return r;
    } catch (e) {
      toast("error", errorMessage(e));
      return undefined;
    }
  }

  const bumpRepo = () =>
    setState((s) => {
      s.repoVersion++;
    });
  const bumpLibrary = () =>
    setState((s) => {
      s.libraryVersion++;
    });

  async function refresh() {
    const st = await backend.state();
    setState((s) => {
      s.settings = st.settings;
      s.workspaces = st.workspaces;
      s.conversations = st.conversations;
      if (s.selectedWorkspace && !st.workspaces.some((w) => w.id === s.selectedWorkspace)) s.selectedWorkspace = null;
      if (!s.selectedWorkspace && st.workspaces.length) s.selectedWorkspace = st.workspaces[0].id;
      if (s.selectedConversation && !st.conversations.some((c) => c.id === s.selectedConversation)) {
        s.selectedConversation = null;
        if (s.view === "conversation") s.view = "workspace";
      }
    });
  }

  async function refreshProposals() {
    const proposals = await backend.proposals();
    setState((s) => {
      s.proposals = proposals;
    });
  }

  function handleEvent(e: HarnessEvent) {
    switch (e.type) {
      case "agent":
        setState((s) => {
          s.chats[e.conversation_id] = applyEvent(s.chats[e.conversation_id] ?? emptyChat(), e.event);
        });
        break;
      case "status":
        setState((s) => {
          const c = s.conversations.find((c) => c.id === e.conversation_id);
          if (c) c.status = e.status;
        });
        break;
      case "committed":
        bumpRepo();
        break;
      case "proposal_created":
        setState((s) => {
          if (!s.proposals.some((p) => p.id === e.proposal.id)) s.proposals.unshift(e.proposal);
        });
        toast("info", `New proposal: ${e.proposal.title}`);
        break;
      case "proposal_failed":
        toast("error", `A ${e.kind} proposal was rejected automatically: ${e.error}`);
        break;
      case "build_output":
        setState((s) => {
          const lines = (s.buildOutput[e.template] ??= []);
          lines.push(e.line);
          if (lines.length > 500) lines.splice(0, lines.length - 500);
        });
        break;
      case "progress":
        setState((s) => {
          s.progress = e.message;
        });
        break;
    }
  }

  let unsubscribe: (() => void) | null = null;

  const actions = {
    toast,
    dismissToast,
    async init() {
      setState((s) => {
        s.loading = true;
      });
      unsubscribe ??= backend.subscribe(handleEvent);
      try {
        const info = await backend.init();
        setState((s) => {
          s.info = info;
        });
        if (info.ready) await Promise.all([refresh(), refreshProposals()]);
      } catch (e) {
        setState((s) => {
          s.info = { ready: false, engine: null, data_dir: "", error: errorMessage(e) };
        });
      } finally {
        setState((s) => {
          s.loading = false;
        });
      }
    },
    dispose() {
      unsubscribe?.();
      unsubscribe = null;
    },
    refresh: () => attempt(refresh),
    setView(view: View) {
      setState((s) => {
        s.view = view;
      });
      if (view === "proposals") void attempt(refreshProposals);
    },
    setTab(tab: ConversationTab) {
      setState((s) => {
        s.tab = tab;
      });
    },
    selectWorkspace(id: string) {
      setState((s) => {
        s.selectedWorkspace = id;
        s.selectedConversation = null;
        s.view = "workspace";
      });
    },
    async selectConversation(id: string) {
      const conv = state.conversations.find((c) => c.id === id);
      setState((s) => {
        s.selectedConversation = id;
        if (conv) s.selectedWorkspace = conv.workspace_id;
        s.view = "conversation";
        s.tab = "chat";
      });
      if (!state.chats[id]) {
        const entries = await attempt(() => backend.transcript(id));
        if (entries)
          setState((s) => {
            // A turn may have started streaming meanwhile; keep it.
            if (!s.chats[id]) s.chats[id] = fromTranscript(entries);
          });
      }
    },
    async addWorkspace(path: string, name: string | null) {
      const ws = await attempt(() => backend.addWorkspace(path, name), "Workspace added");
      if (ws) {
        await refresh();
        actions.selectWorkspace(ws.id);
      }
      return ws;
    },
    async removeWorkspace(id: string) {
      const ok = await attempt(() => backend.removeWorkspace(id).then(() => true), "Workspace removed");
      if (ok) await refresh();
      return !!ok;
    },
    /** Returns the error message instead of toasting it, for inline display. */
    async configureWorkspace(id: string, templates: string[], network: NetworkPolicy): Promise<string | null> {
      try {
        await backend.configureWorkspace(id, templates, network);
        await refresh();
        toast("success", "Workspace saved");
        return null;
      } catch (e) {
        return errorMessage(e);
      }
    },
    async buildTemplates(id: string) {
      setState((s) => {
        s.buildOutput = {};
      });
      await attempt(() => backend.buildTemplates(id), "Templates are up to date");
      setState((s) => {
        s.progress = null;
      });
    },
    async createConversation(workspaceId: string, base: string, title: string) {
      const conv = await attempt(() => backend.createConversation(workspaceId, base, title));
      setState((s) => {
        s.progress = null;
      });
      if (conv) {
        setState((s) => {
          s.chats[conv.id] = emptyChat();
        });
        await refresh();
        await actions.selectConversation(conv.id);
        bumpRepo();
      }
      return conv;
    },
    async send(id: string, prompt: string) {
      const text = prompt.trim();
      if (!text) return;
      setState((s) => {
        s.chats[id] = startTurn(s.chats[id] ?? emptyChat(), text);
      });
      try {
        await backend.sendMessage(id, text);
      } catch (e) {
        const message = errorMessage(e);
        setState((s) => {
          s.chats[id] = applyEvent(s.chats[id] ?? emptyChat(), { type: "error", message });
        });
      }
      await attempt(refresh);
      bumpRepo();
    },
    cancel: (id: string) => attempt(() => backend.cancel(id)),
    restartSandbox: (id: string) => attempt(() => backend.restartSandbox(id), "Sandbox restarted with the current settings"),
    async rename(id: string, title: string) {
      if (!title.trim()) return;
      await attempt(() => backend.renameConversation(id, title));
      await attempt(refresh);
    },
    /** Returns the outcome so the dialog can ask for confirmation. */
    async deleteConversation(id: string, mode: DeleteMode): Promise<DeleteOutcome | undefined> {
      const out = await attempt(() => backend.deleteConversation(id, mode));
      if (out?.result === "deleted") {
        setState((s) => {
          delete s.chats[id];
        });
        toast("success", "Conversation deleted");
        await refresh();
        bumpRepo();
      }
      return out;
    },
    async merge(id: string, into: string): Promise<MergeOutcome | undefined> {
      const out = await attempt(() => backend.mergeConversation(id, into));
      if (out) {
        if (out.kind === "conflicts") toast("error", `Merge conflicts in ${out.paths.join(", ")}`);
        else if (out.kind === "up_to_date") toast("info", `${into} already contains these changes`);
        else toast("success", `Merged into ${into}`);
        bumpRepo();
      }
      return out;
    },
    async fetchRemotes(workspaceId: string, remote: string | null) {
      const ok = await attempt(() => backend.fetch(workspaceId, remote).then(() => true), remote ? `Fetched ${remote}` : "Fetched all remotes");
      if (ok) bumpRepo();
    },
    async push(workspaceId: string, branch: string, remote: string) {
      const ok = await attempt(() => backend.push(workspaceId, branch, remote).then(() => true), `Pushed ${branch} to ${remote}`);
      if (ok) bumpRepo();
    },
    async updateFromBase(id: string) {
      const out = await attempt(() => backend.updateFromBase(id));
      if (out) {
        if (out.kind === "up_to_date") toast("info", "Already up to date with the base branch");
        else if (out.kind === "conflicts") toast("error", `The update has conflicts in ${out.paths.join(", ")}. Ask the agent to resolve them or abort.`);
        else toast("success", "Updated from the base branch");
        bumpRepo();
      }
      return out;
    },
    async rebase(id: string) {
      const out = await attempt(() => backend.rebaseConversation(id));
      if (out) {
        if (out.kind === "up_to_date") toast("info", "Already based on the latest base branch");
        else if (out.kind === "conflicts") toast("error", `Rebasing would conflict in ${out.paths.join(", ")}; nothing changed. Use Update from base instead.`);
        else toast("success", "Rebased onto the base branch");
        bumpRepo();
      }
      return out;
    },
    async abortUpdate(id: string) {
      await attempt(() => backend.abortUpdate(id), "Update aborted");
      bumpRepo();
    },
    async resolveConflicts(id: string) {
      setState((s) => {
        s.tab = "chat";
        s.chats[id] = startTurn(s.chats[id] ?? emptyChat(), "Resolve the conflicts from the update");
      });
      try {
        await backend.resolveConflicts(id);
      } catch (e) {
        const message = errorMessage(e);
        setState((s) => {
          s.chats[id] = applyEvent(s.chats[id] ?? emptyChat(), { type: "error", message });
        });
      }
      await attempt(refresh);
      bumpRepo();
    },
    async saveSettings(settings: Settings) {
      const ok = await attempt(() => backend.updateSettings(settings).then(() => true), "Settings saved");
      if (ok) await attempt(refresh);
      return !!ok;
    },
    refreshProposals: () => attempt(refreshProposals),
    async approve(kind: LibraryKind, id: string) {
      const ok = await attempt(() => backend.approve(kind, id), "Proposal approved");
      await attempt(refreshProposals);
      bumpLibrary();
      return ok;
    },
    async reject(kind: LibraryKind, id: string) {
      await attempt(() => backend.reject(kind, id), "Proposal rejected");
      await attempt(refreshProposals);
    },
    async markLastTurnWrong(id: string) {
      const skills = await attempt(() => backend.markLastTurnWrong(id));
      if (skills) toast("info", skills.length ? `Recorded a bad outcome for ${skills.join(", ")}` : "That turn used no skills");
      await attempt(refresh);
      bumpLibrary();
    },
    /** Submit a user-authored proposal; returns true when it was created. */
    async propose(kind: "skill" | "skill-removal" | "template", text: string, rationale: string): Promise<boolean> {
      const created = await attempt(() =>
        kind === "skill" ? backend.proposeSkill(text, rationale) : kind === "template" ? backend.proposeTemplate(text, rationale) : backend.proposeSkillRemoval(text, rationale),
      );
      if (!created) return false;
      toast("success", `Proposal created: ${created.title}. Review it under Proposals.`);
      await attempt(refreshProposals);
      return true;
    },
    async revert(kind: LibraryKind, commit: string) {
      await attempt(() => backend.revert(kind, commit), "Change reverted");
      bumpLibrary();
    },
    async cleanup() {
      const r = await attempt(() => backend.cleanupOrphans());
      if (r) {
        const n = r.containers.length + r.worktrees.length + r.branches.length;
        toast("success", n ? `Removed ${n} orphaned resources` : "Nothing to clean up");
        if (r.errors.length) toast("error", `Could not remove:\n${r.errors.join("\n")}`);
        await attempt(refresh);
      }
    },
    async buildImage() {
      toast("info", "Building the agent image, this can take a few minutes");
      await attempt(() => backend.buildAgentImage(), "Agent image built");
    },
  };

  return { state, actions, backend };
}

export type App = ReturnType<typeof createApp>;

export const AppContext = createContext<App>();

export function useApp(): App {
  return useContext(AppContext);
}

export function workspaceOf(state: UiState): Workspace | undefined {
  return state.workspaces.find((w) => w.id === state.selectedWorkspace);
}

export function conversationOf(state: UiState): Conversation | undefined {
  return state.conversations.find((c) => c.id === state.selectedConversation);
}
