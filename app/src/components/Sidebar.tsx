import { createMemo, createSignal, For, Show } from "solid-js";
import type { Conversation } from "../api/types";
import { useApp } from "../store";
import { Badge, Button, inputClass } from "./ui";

export function Sidebar() {
  const { state, actions, backend } = useApp();
  const [adding, setAdding] = createSignal(false);
  const [path, setPath] = createSignal("");
  const conversations = createMemo(() => state.conversations.filter((c) => c.workspace_id === state.selectedWorkspace));
  const pending = createMemo(() => state.proposals.length);

  const browse = async () => {
    const dir = await backend.pickDirectory();
    if (dir) setPath(dir);
  };
  const add = async (e: SubmitEvent) => {
    e.preventDefault();
    if (!path().trim()) return;
    const ws = await actions.addWorkspace(path().trim(), null);
    if (ws) {
      setPath("");
      setAdding(false);
    }
  };

  return (
    <nav class="flex w-64 shrink-0 flex-col border-r border-zinc-800 bg-zinc-900/60" data-testid="sidebar">
      <div class="flex items-center justify-between px-4 py-3">
        <span class="font-semibold tracking-tight text-zinc-100">nucleus</span>
        <Show when={state.info?.engine}>{(e) => <Badge tone="sky">{e()}</Badge>}</Show>
      </div>

      <Section title="Workspaces" action={<Button variant="ghost" size="sm" aria-label="Add workspace" data-testid="add-workspace" onClick={() => setAdding((v) => !v)}>+</Button>}>
        <Show when={adding()}>
          <form class="flex flex-col gap-2 px-3 pb-2" onSubmit={add} data-testid="add-workspace-form">
            <input class={inputClass} placeholder="/path/to/git/repo" value={path()} onInput={(e) => setPath(e.currentTarget.value)} aria-label="Repository path" />
            <div class="flex gap-2">
              <Button type="submit" size="sm" disabled={!path().trim()}>
                Add
              </Button>
              <Button variant="secondary" size="sm" onClick={browse}>
                Browse…
              </Button>
            </div>
          </form>
        </Show>
        <For each={state.workspaces}>
          {(ws) => (
            <NavItem active={state.selectedWorkspace === ws.id && state.view === "workspace"} onClick={() => actions.selectWorkspace(ws.id)} testid={`workspace-${ws.name}`}>
              <span class="truncate">{ws.name}</span>
            </NavItem>
          )}
        </For>
      </Section>

      <Show when={state.selectedWorkspace}>
        <Section title="Conversations">
          <For each={conversations()} fallback={<p class="px-4 py-1 text-xs text-zinc-600">No conversations</p>}>
            {(c) => (
              <NavItem active={state.selectedConversation === c.id && state.view === "conversation"} onClick={() => void actions.selectConversation(c.id)} testid={`conversation-${c.id}`}>
                <StatusDot status={c.status} />
                <span class="truncate">{c.title}</span>
              </NavItem>
            )}
          </For>
        </Section>
      </Show>

      <div class="mt-auto border-t border-zinc-800 py-2">
        <NavItem active={state.view === "proposals"} onClick={() => actions.setView("proposals")} testid="nav-proposals">
          <span class="flex-1">Proposals</span>
          <Show when={pending() > 0}>
            <Badge tone="amber" testid="proposal-count">
              {pending()}
            </Badge>
          </Show>
        </NavItem>
        <NavItem active={state.view === "skills"} onClick={() => actions.setView("skills")} testid="nav-skills">
          Skills & tools
        </NavItem>
        <NavItem active={state.view === "logs"} onClick={() => actions.setView("logs")} testid="nav-logs">
          Logs
        </NavItem>
        <NavItem active={state.view === "settings"} onClick={() => actions.setView("settings")} testid="nav-settings">
          Settings
        </NavItem>
      </div>
    </nav>
  );
}

function Section(props: { title: string; action?: import("@solidjs/web").JSX.Element; children: import("@solidjs/web").JSX.Element }) {
  return (
    <div class="py-2">
      <div class="flex items-center justify-between px-4 pb-1">
        <span class="text-[11px] font-semibold tracking-wider text-zinc-500 uppercase">{props.title}</span>
        {props.action}
      </div>
      {props.children}
    </div>
  );
}

function NavItem(props: { active: boolean; onClick: () => void; testid?: string; children: import("@solidjs/web").JSX.Element }) {
  return (
    <button
      type="button"
      data-testid={props.testid}
      aria-current={props.active ? "page" : undefined}
      onClick={() => props.onClick()}
      class={["flex w-full items-center gap-2 px-4 py-1.5 text-left text-sm", props.active ? "bg-zinc-800 text-zinc-50" : "text-zinc-400 hover:bg-zinc-800/50 hover:text-zinc-200"]}
    >
      {props.children}
    </button>
  );
}

export function StatusDot(props: { status: Conversation["status"] }) {
  return (
    <span
      data-testid="status-dot"
      data-status={props.status}
      title={props.status}
      class={["size-2 shrink-0 rounded-full", { "bg-zinc-600": props.status === "idle", "animate-pulse bg-indigo-400": props.status === "running", "bg-red-500": props.status === "error" }]}
    />
  );
}
