import { createMemo, createSignal, Errored, For, Loading, Match, Show, Switch } from "solid-js";
import type { Conversation } from "../api/types";
import { emptyChat, toolSummary, type ChatItem } from "../lib/chat";
import { formatCost, relativeTime, shortId } from "../lib/format";
import { useApp } from "../store";
import { DeleteDialog } from "./DeleteDialog";
import { NetworkView } from "./NetworkView";
import { DiffView } from "./DiffView";
import { StatusDot } from "./Sidebar";
import { Badge, Button, ErrorBox, errorText, inputBase, inputClass, Spinner } from "./ui";
import { Tab } from "./WorkspaceView";

export function ConversationView(props: { conversation: Conversation }) {
  const { state, actions } = useApp();
  const [deleting, setDeleting] = createSignal(false);
  const [editing, setEditing] = createSignal(false);
  const [title, setTitle] = createSignal("");
  const saveTitle = async (e: SubmitEvent) => {
    e.preventDefault();
    await actions.rename(props.conversation.id, title());
    setEditing(false);
  };
  return (
    <div class="flex min-h-0 flex-1 flex-col" data-testid="conversation-view">
      <header class="flex items-center gap-3 border-b border-zinc-800 px-6 py-3">
        <StatusDot status={props.conversation.status} />
        <div class="min-w-0 flex-1">
          <Show
            when={editing()}
            fallback={
              <h1
                class="cursor-text truncate text-lg font-semibold text-zinc-100"
                title="Click to rename"
                data-testid="conversation-title"
                onClick={() => {
                  setTitle(props.conversation.title);
                  setEditing(true);
                }}
              >
                {props.conversation.title}
              </h1>
            }
          >
            <form onSubmit={saveTitle} class="flex gap-2">
              <input class={inputClass} value={title()} onInput={(e) => setTitle(e.currentTarget.value)} aria-label="Conversation title" ref={(el) => queueMicrotask(() => el.focus())} />
              <Button type="submit" size="sm">
                Save
              </Button>
              <Button size="sm" variant="ghost" onClick={() => setEditing(false)}>
                Cancel
              </Button>
            </form>
          </Show>
          <p class="flex items-center gap-2 text-xs text-zinc-500">
            <Badge tone="indigo">{props.conversation.branch}</Badge>
            <span>from {props.conversation.base_branch}</span>
            <span>· {relativeTime(props.conversation.created)}</span>
          </p>
        </div>
        <Button
          variant="ghost"
          size="sm"
          title="Recreate the sandbox to apply new limits, network or templates"
          disabled={props.conversation.status === "running"}
          onClick={() => void actions.restartSandbox(props.conversation.id)}
          data-testid="restart-sandbox"
        >
          Restart sandbox
        </Button>
        <Button variant="ghost" size="sm" onClick={() => setDeleting(true)} data-testid="delete-conversation">
          Delete
        </Button>
      </header>
      <div class="flex gap-1 border-b border-zinc-800 px-6" role="tablist">
        <Tab active={state.tab === "chat"} onClick={() => actions.setTab("chat")} label="Chat" />
        <Tab active={state.tab === "changes"} onClick={() => actions.setTab("changes")} label="Changes" />
        <Tab active={state.tab === "network"} onClick={() => actions.setTab("network")} label="Network" />
      </div>
      <Switch>
        <Match when={state.tab === "chat"}>
          <ChatView conversation={props.conversation} />
        </Match>
        <Match when={state.tab === "changes"}>
          <ChangesView conversation={props.conversation} />
        </Match>
        <Match when={state.tab === "network"}>
          <NetworkView conversation={props.conversation} />
        </Match>
      </Switch>
      <Show when={deleting()}>
        <DeleteDialog conversation={props.conversation} onClose={() => setDeleting(false)} />
      </Show>
    </div>
  );
}

function ChatView(props: { conversation: Conversation }) {
  const { state, actions } = useApp();
  const [draft, setDraft] = createSignal("");
  const chat = createMemo(() => state.chats[props.conversation.id] ?? emptyChat());
  const running = createMemo(() => props.conversation.status === "running");
  let list: HTMLDivElement | undefined;
  const send = () => {
    const text = draft();
    if (!text.trim() || running()) return;
    setDraft("");
    void actions.send(props.conversation.id, text);
  };
  return (
    <div class="flex min-h-0 flex-1 flex-col">
      <div class="flex-1 overflow-y-auto px-6 py-4" ref={(el) => (list = el)} data-testid="chat-log">
        <div class="mx-auto flex max-w-3xl flex-col gap-3">
          <For each={chat().items} fallback={<p class="py-10 text-center text-sm text-zinc-500">Describe a task to start. The agent works on its own branch in a sandbox.</p>}>
            {(item) => <ChatEntry item={item} />}
          </For>
          <Show when={running()}>
            <Spinner label="Agent is working…" />
          </Show>
          <Show when={!running() && props.conversation.last_turn_skills.length > 0}>
            <div class="flex items-center gap-2 text-xs text-zinc-500" data-testid="turn-feedback">
              <span>This turn used {props.conversation.last_turn_skills.join(", ")}.</span>
              <button type="button" class="text-red-300 hover:underline" onClick={() => void actions.markLastTurnWrong(props.conversation.id)} data-testid="mark-wrong">
                Mark result as wrong
              </button>
            </div>
          </Show>
        </div>
      </div>
      <form
        class="border-t border-zinc-800 p-4"
        onSubmit={(e) => {
          e.preventDefault();
          send();
          queueMicrotask(() => list?.scrollTo?.({ top: list.scrollHeight }));
        }}
      >
        <div class="mx-auto flex max-w-3xl items-end gap-2">
          <textarea
            class={[inputClass, "min-h-[44px] flex-1 resize-y"]}
            rows={2}
            placeholder="Message the agent (Ctrl+Enter to send)"
            aria-label="Message"
            value={draft()}
            onInput={(e) => setDraft(e.currentTarget.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
                e.preventDefault();
                send();
              }
            }}
          />
          <Show
            when={running()}
            fallback={
              <Button type="submit" disabled={!draft().trim()} data-testid="send">
                Send
              </Button>
            }
          >
            <Button variant="danger" onClick={() => void actions.cancel(props.conversation.id)} data-testid="stop">
              Stop
            </Button>
          </Show>
        </div>
      </form>
    </div>
  );
}

function ChatEntry(props: { item: ChatItem }) {
  const [open, setOpen] = createSignal(false);
  return (
    <Switch>
      <Match when={props.item.kind === "user" && props.item}>
        {(it) => (
          <div class="self-end rounded-lg bg-indigo-600/20 px-3 py-2 text-sm whitespace-pre-wrap text-zinc-100" data-testid="msg-user">
            {it().text}
          </div>
        )}
      </Match>
      <Match when={props.item.kind === "assistant" && props.item}>
        {(it) => (
          <div class="text-sm leading-relaxed whitespace-pre-wrap text-zinc-200" data-testid="msg-assistant" data-streaming={it().streaming ? "true" : "false"}>
            {it().text}
            <Show when={it().streaming}>
              <span class="ml-0.5 inline-block h-4 w-1.5 animate-pulse bg-zinc-400 align-middle" />
            </Show>
          </div>
        )}
      </Match>
      <Match when={props.item.kind === "thinking" && props.item}>
        {(it) => (
          <div class="border-l-2 border-zinc-700 pl-3 text-xs whitespace-pre-wrap text-zinc-500 italic" data-testid="msg-thinking">
            {it().text}
          </div>
        )}
      </Match>
      <Match when={props.item.kind === "tool" && props.item}>
        {(it) => (
          <div class="rounded-md border border-zinc-800 bg-zinc-900/60 text-xs" data-testid="msg-tool">
            <button type="button" class="flex w-full items-center gap-2 px-3 py-1.5 text-left" onClick={() => setOpen((v) => !v)} aria-expanded={open() ? "true" : "false"}>
              <Badge tone={it().isError ? "red" : it().result === null ? "amber" : "zinc"}>{it().name}</Badge>
              <span class="flex-1 truncate font-mono text-zinc-400">{toolSummary(it().name, it().input)}</span>
              <span class="text-zinc-600">{open() ? "▾" : "▸"}</span>
            </button>
            <Show when={open()}>
              <pre class="max-h-60 overflow-auto border-t border-zinc-800 px-3 py-2 font-mono whitespace-pre-wrap text-zinc-400">{JSON.stringify(it().input, null, 2)}</pre>
              <Show when={it().result !== null}>
                <pre class={["max-h-80 overflow-auto border-t border-zinc-800 px-3 py-2 font-mono whitespace-pre-wrap", it().isError ? "text-red-300" : "text-zinc-300"]} data-testid="tool-result">
                  {it().result}
                </pre>
              </Show>
            </Show>
          </div>
        )}
      </Match>
      <Match when={props.item.kind === "error" && props.item}>{(it) => <ErrorBox message={it().message} testid="msg-error" />}</Match>
      <Match when={props.item.kind === "turn" && props.item}>
        {(it) => (
          <div class="flex items-center gap-2 text-[11px] text-zinc-600" data-testid="msg-turn">
            <span class="h-px flex-1 bg-zinc-800" />
            <span>{it().isError ? "turn failed" : "turn complete"}</span>
            <Show when={formatCost(it().costUsd)}>{(c) => <span>· {c()}</span>}</Show>
            <Show when={it().durationMs}>{(d) => <span>· {(d() / 1000).toFixed(1)}s</span>}</Show>
            <span class="h-px flex-1 bg-zinc-800" />
          </div>
        )}
      </Match>
    </Switch>
  );
}

function ChangesView(props: { conversation: Conversation }) {
  const { state, actions, backend } = useApp();
  const [target, setTarget] = createSignal<string | null>(null);
  const [merging, setMerging] = createSignal(false);
  const data = createMemo(async () => {
    void state.repoVersion;
    const [diff, unmerged, branches] = await Promise.all([
      backend.conversationDiff(props.conversation.id),
      backend.unmergedCommits(props.conversation.id),
      backend.branches(props.conversation.workspace_id),
    ]);
    return { diff, unmerged, targets: branches.filter((b) => b.kind === "local").map((b) => b.name) };
  });
  const defaultTarget = (targets: string[]) => (targets.includes(props.conversation.base_branch) ? props.conversation.base_branch : (targets[0] ?? ""));
  const merge = async (fallback: string) => {
    setMerging(true);
    await actions.merge(props.conversation.id, target() ?? fallback);
    setMerging(false);
  };
  return (
    <div class="flex-1 overflow-y-auto px-6 py-4" data-testid="changes-view">
      <Errored fallback={(err) => <ErrorBox message={errorText(err())} />}>
        <Loading fallback={<Spinner label="Loading changes…" />}>
          <div class="mb-4 flex flex-wrap items-center gap-2 rounded-lg border border-zinc-800 bg-zinc-900/50 p-3 text-sm">
            <span class="text-zinc-400">
              {data().unmerged.length} {data().unmerged.length === 1 ? "commit" : "commits"} not on any local or origin branch
            </span>
            <span class="flex-1" />
            <label class="flex items-center gap-2 whitespace-nowrap text-zinc-400">
              Merge into
              <select class={[inputBase, "w-44"]} onChange={(e) => setTarget(e.currentTarget.value)} aria-label="Merge target">
                <For each={data().targets}>
                  {(b) => (
                    <option value={b} selected={b === (target() ?? defaultTarget(data().targets))}>
                      {b}
                    </option>
                  )}
                </For>
              </select>
            </label>
            <Button size="sm" disabled={merging() || !data().targets.length} onClick={() => void merge(defaultTarget(data().targets))} data-testid="merge">
              {merging() ? "Merging…" : "Merge"}
            </Button>
          </div>
          <Show when={data().unmerged.length}>
            <ul class="mb-4 flex flex-col gap-1 text-sm" data-testid="unmerged-commits">
              <For each={data().unmerged}>
                {(c) => (
                  <li class="flex gap-2">
                    <span class="font-mono text-xs text-zinc-600">{shortId(c.id)}</span>
                    <span class="truncate text-zinc-300">{c.summary}</span>
                  </li>
                )}
              </For>
            </ul>
          </Show>
          <DiffView files={data().diff} emptyText="The agent has not changed anything yet" />
        </Loading>
      </Errored>
    </div>
  );
}
