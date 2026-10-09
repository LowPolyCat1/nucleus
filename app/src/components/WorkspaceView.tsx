import { createMemo, createSignal, Errored, For, Loading, Show } from "solid-js";
import type { NetworkPolicy, Workspace } from "../api/types";
import { describePolicy } from "../lib/network";
import { useApp } from "../store";
import { BranchTree } from "./BranchTree";
import { StreamedDiff } from "./DiffView";
import { NetworkEditor } from "./NetworkEditor";
import { Badge, Button, ErrorBox, errorText, inputBase, inputClass, Modal, Spinner } from "./ui";

export function WorkspaceView(props: { workspace: Workspace }) {
  const { state, actions, backend } = useApp();
  const [tab, setTab] = createSignal<"branches" | "setup">("branches");
  const branches = createMemo(async () => {
    void state.repoVersion;
    return backend.branches(props.workspace.id);
  });

  return (
    <div class="flex min-h-0 flex-1 flex-col" data-testid="workspace-view">
      <header class="flex items-center gap-3 border-b border-zinc-800 px-6 py-3">
        <div class="min-w-0 flex-1">
          <h1 class="truncate text-lg font-semibold text-zinc-100">{props.workspace.name}</h1>
          <p class="truncate font-mono text-xs text-zinc-500">{props.workspace.repo}</p>
        </div>
        <Badge tone="zinc">{describePolicy(props.workspace.network)}</Badge>
        <Button variant="ghost" size="sm" data-testid="remove-workspace" onClick={() => void actions.removeWorkspace(props.workspace.id)}>
          Remove
        </Button>
      </header>
      <div class="flex-1 overflow-y-auto px-6 py-4">
        <Errored fallback={(err) => <ErrorBox message={errorText(err())} />}>
          <Loading fallback={<Spinner label="Loading branches…" />}>
            <NewConversation workspaceId={props.workspace.id} branches={branches().filter((b) => b.kind !== "agent").map((b) => b.name)} head={branches().find((b) => b.is_head)?.name ?? "main"} />
          </Loading>
        </Errored>
        <div class="mt-6 mb-3 flex gap-1 border-b border-zinc-800" role="tablist">
          <Tab active={tab() === "branches"} onClick={() => setTab("branches")} label="Branches" />
          <Tab active={tab() === "setup"} onClick={() => setTab("setup")} label="Templates & network" />
        </div>
        <Show when={tab() === "branches"} fallback={<WorkspaceSetup workspace={props.workspace} />}>
          <Branches workspaceId={props.workspace.id} />
        </Show>
      </div>
    </div>
  );
}

export function Tab(props: { active: boolean; onClick: () => void; label: string }) {
  return (
    <button
      type="button"
      role="tab"
      aria-selected={props.active ? "true" : "false"}
      onClick={() => props.onClick()}
      class={["-mb-px border-b-2 px-3 py-1.5 text-sm", props.active ? "border-indigo-400 text-zinc-100" : "border-transparent text-zinc-500 hover:text-zinc-300"]}
    >
      {props.label}
    </button>
  );
}

function NewConversation(props: { workspaceId: string; branches: string[]; head: string }) {
  const { actions } = useApp();
  const [base, setBase] = createSignal<string | null>(null);
  const [title, setTitle] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const submit = async (e: SubmitEvent) => {
    e.preventDefault();
    setBusy(true);
    await actions.createConversation(props.workspaceId, base() ?? props.head, title());
    setBusy(false);
  };
  return (
    <form class="flex flex-wrap items-end gap-2 rounded-lg border border-zinc-800 bg-zinc-900/50 p-4" onSubmit={submit} data-testid="new-conversation">
      <label class="flex min-w-60 flex-1 flex-col gap-1 text-xs text-zinc-400">
        New conversation
        <input class={inputClass} placeholder="What should the agent do?" value={title()} onInput={(e) => setTitle(e.currentTarget.value)} aria-label="Conversation title" />
      </label>
      <label class="flex flex-col gap-1 text-xs text-zinc-400">
        Base branch
        <select class={inputClass} onChange={(e) => setBase(e.currentTarget.value)} aria-label="Base branch">
          <For each={props.branches}>
            {(b) => (
              <option value={b} selected={b === (base() ?? props.head)}>
                {b}
              </option>
            )}
          </For>
        </select>
      </label>
      <Button type="submit" disabled={busy()} data-testid="start-conversation">
        {busy() ? "Starting…" : "Start"}
      </Button>
    </form>
  );
}

function Branches(props: { workspaceId: string }) {
  const { state, backend } = useApp();
  const [from, setFrom] = createSignal("");
  const [to, setTo] = createSignal("");
  const [compare, setCompare] = createSignal<[string, string] | null>(null);
  const data = createMemo(async () => {
    void state.repoVersion;
    const [branches, commits] = await Promise.all([backend.branches(props.workspaceId), backend.graph(props.workspaceId, 300)]);
    return { branches, commits };
  });

  const pick = (name: string) => {
    if (!from() || (from() && to())) {
      setFrom(name);
      setTo("");
    } else setTo(name);
  };
  return (
    <Errored fallback={(err) => <ErrorBox message={errorText(err())} />}>
      <Loading fallback={<Spinner label="Loading history…" />}>
        <RemoteBar workspaceId={props.workspaceId} localBranches={data().branches.filter((b) => b.kind === "local").map((b) => b.name)} />
        <div class="mb-3 flex flex-wrap items-center gap-2 text-sm" data-testid="compare">
          <span class="text-zinc-500">Compare</span>
          <select class={[inputBase, "w-48"]} onChange={(e) => setFrom(e.currentTarget.value)} aria-label="Compare from">
            <option value="" selected={!from()}>
              base…
            </option>
            <For each={data().branches}>
              {(b) => (
                <option value={b.name} selected={b.name === from()}>
                  {b.name}
                </option>
              )}
            </For>
          </select>
          <span class="text-zinc-600">→</span>
          <select class={[inputBase, "w-48"]} onChange={(e) => setTo(e.currentTarget.value)} aria-label="Compare to">
            <option value="" selected={!to()}>
              target…
            </option>
            <For each={data().branches}>
              {(b) => (
                <option value={b.name} selected={b.name === to()}>
                  {b.name}
                </option>
              )}
            </For>
          </select>
          <Button size="sm" variant="secondary" disabled={!from() || !to()} onClick={() => setCompare([from(), to()])} data-testid="compare-button">
            Show diff
          </Button>
          <Show when={compare()}>
            <Button size="sm" variant="ghost" onClick={() => setCompare(null)}>
              Clear
            </Button>
          </Show>
        </div>
        <Show when={compare()}>
          <div class="mb-4">
            <Show when={compare()}>
              {(c) => (
                <StreamedDiff
                  load={(onFile) => backend.diffStream(props.workspaceId, c()[0], c()[1], onFile)}
                  streamKey={`${c()[0]}..${c()[1]}:${state.repoVersion}`}
                  emptyText="The branches have the same content"
                />
              )}
            </Show>
          </div>
        </Show>
        <BranchTree commits={data().commits} branches={data().branches} onSelectBranch={pick} />
      </Loading>
    </Errored>
  );
}

function WorkspaceSetup(props: { workspace: Workspace }) {
  const { state, actions, backend } = useApp();
  const [log, setLog] = createSignal<{ name: string; text: string } | null>(null);
  const showLog = async (name: string) => {
    try {
      const text = await backend.templateBuildLog(name);
      setLog({ name, text: text ?? "This template has not been built yet." });
    } catch (e) {
      actions.toast("error", errorText(e));
    }
  };
  const [selected, setSelected] = createSignal<string[]>([...props.workspace.templates]);
  const [network, setNetwork] = createSignal<NetworkPolicy | null>(props.workspace.network);
  const [error, setError] = createSignal<string | null>(null);
  const [version, setVersion] = createSignal(0);
  const available = createMemo(() => backend.availableTemplates());
  const status = createMemo(async () => {
    void version();
    return backend.templateStatus(props.workspace.id);
  });

  const toggle = (name: string) => setSelected((s) => (s.includes(name) ? s.filter((x) => x !== name) : [...s, name]));
  const move = (name: string, delta: number) =>
    setSelected((s) => {
      const i = s.indexOf(name);
      const j = i + delta;
      if (i < 0 || j < 0 || j >= s.length) return s;
      const next = [...s];
      [next[i], next[j]] = [next[j], next[i]];
      return next;
    });
  const save = async () => {
    const n = network();
    if (!n) return;
    setError(await actions.configureWorkspace(props.workspace.id, selected(), n));
    setVersion((v) => v + 1);
  };
  const build = async () => {
    await actions.buildTemplates(props.workspace.id);
    setVersion((v) => v + 1);
  };

  return (
    <div class="grid gap-6 lg:grid-cols-2" data-testid="workspace-setup">
      <section>
        <h3 class="mb-1 text-sm font-semibold text-zinc-200">Dependency templates</h3>
        <p class="mb-3 text-xs text-zinc-500">Prebuilt dependency directories mounted into every sandbox, in this order. The first template wins on PATH clashes.</p>
        <Errored fallback={(err) => <ErrorBox message={errorText(err())} />}>
          <Loading fallback={<Spinner />}>
            <ul class="flex flex-col gap-1">
              <For each={available()} fallback={<p class="text-sm text-zinc-500">No templates in the library yet. The agent can propose some.</p>}>
                {(t) => (
                  <li class="flex items-center gap-2 rounded-md border border-zinc-800 px-3 py-2 text-sm" data-testid={`template-${t.name}`}>
                    <input type="checkbox" checked={selected().includes(t.name)} onChange={() => toggle(t.name)} aria-label={`Use template ${t.name}`} />
                    <span class="flex-1">
                      <span class="font-medium text-zinc-200">{t.name}</span>
                      <span class="ml-2 text-xs text-zinc-500">{t.description}</span>
                    </span>
                    <Badge>{t.mount.mode}</Badge>
                    <Show when={selected().includes(t.name)}>
                      <span class="font-mono text-xs text-zinc-500">#{selected().indexOf(t.name) + 1}</span>
                      <Button size="sm" variant="ghost" aria-label={`Move ${t.name} up`} onClick={() => move(t.name, -1)}>
                        ↑
                      </Button>
                      <Button size="sm" variant="ghost" aria-label={`Move ${t.name} down`} onClick={() => move(t.name, 1)}>
                        ↓
                      </Button>
                    </Show>
                  </li>
                )}
              </For>
            </ul>
          </Loading>
        </Errored>
        <div class="mt-4">
          <h4 class="mb-2 text-xs font-semibold tracking-wider text-zinc-500 uppercase">Build status</h4>
          <Errored fallback={(err) => <ErrorBox message={errorText(err())} />}>
            <Loading fallback={<Spinner />}>
              <ul class="flex flex-col gap-1 text-sm" data-testid="template-status">
                <For each={status()} fallback={<li class="text-zinc-500">No templates configured</li>}>
                  {(s) => (
                    <li class="flex items-center gap-2">
                      <span class="flex-1">{s.name}</span>
                      <Button size="sm" variant="ghost" onClick={() => void showLog(s.name)} data-testid={`build-log-${s.name}`}>
                        Log
                      </Button>
                      <Show when={s.error} fallback={<Badge tone={s.fresh ? "emerald" : "amber"}>{s.fresh ? "built" : "needs build"}</Badge>}>
                        <Badge tone="red">error</Badge>
                      </Show>
                    </li>
                  )}
                </For>
              </ul>
            </Loading>
          </Errored>
          <Button class="mt-2" size="sm" variant="secondary" onClick={build} data-testid="build-templates">
            Build templates
          </Button>
          <Show when={Object.keys(state.buildOutput).length > 0}>
            <pre class="mt-3 max-h-60 overflow-auto rounded-md border border-zinc-800 bg-zinc-950 p-2 font-mono text-[11px] text-zinc-400" data-testid="build-output">
              <For each={Object.entries(state.buildOutput)}>{([name, lines]) => <>{lines.map((l) => `[${name}] ${l}\n`).join("")}</>}</For>
            </pre>
          </Show>
        </div>
      </section>
      <section>
        <h3 class="mb-1 text-sm font-semibold text-zinc-200">Network policy</h3>
        <p class="mb-3 text-xs text-zinc-500">Applies to new sandboxes of this workspace.</p>
        <NetworkEditor name="workspace" value={props.workspace.network} onChange={setNetwork} />
      </section>
      <div class="lg:col-span-2">
        <Show when={error()}>{(e) => <ErrorBox message={e()} testid="workspace-error" />}</Show>
        <Button class="mt-2" onClick={save} disabled={!network()} data-testid="save-workspace">
          Save
        </Button>
      </div>
      <Show when={log()}>
        {(l) => (
          <Modal title={`Last build of ${l().name}`} onClose={() => setLog(null)} testid="build-log">
            <pre class="max-h-[60vh] overflow-auto font-mono text-xs whitespace-pre-wrap text-zinc-300">{l().text || "(no output)"}</pre>
          </Modal>
        )}
      </Show>
    </div>
  );
}

function RemoteBar(props: { workspaceId: string; localBranches: string[] }) {
  const { actions, backend } = useApp();
  const remotes = createMemo(() => backend.remotes(props.workspaceId));
  const [branch, setBranch] = createSignal<string | null>(null);
  const [remote, setRemote] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal(false);
  const run = async (f: () => Promise<unknown>) => {
    setBusy(true);
    await f();
    setBusy(false);
  };
  return (
    <Show when={remotes().length > 0}>
      <div class="mb-3 flex flex-wrap items-center gap-2 text-sm" data-testid="remote-bar">
        <Button size="sm" variant="secondary" disabled={busy()} onClick={() => void run(() => actions.fetchRemotes(props.workspaceId, null))} data-testid="fetch">
          Fetch
        </Button>
        <span class="ml-4 text-zinc-500">Push</span>
        <select class={[inputBase, "w-44"]} aria-label="Branch to push" onChange={(e) => setBranch(e.currentTarget.value)}>
          <For each={props.localBranches}>
            {(b) => (
              <option value={b} selected={b === (branch() ?? props.localBranches[0])}>
                {b}
              </option>
            )}
          </For>
        </select>
        <span class="text-zinc-600">to</span>
        <select class={[inputBase, "w-32"]} aria-label="Remote" onChange={(e) => setRemote(e.currentTarget.value)}>
          <For each={remotes()}>
            {(r) => (
              <option value={r} selected={r === (remote() ?? remotes()[0])}>
                {r}
              </option>
            )}
          </For>
        </select>
        <Button
          size="sm"
          variant="secondary"
          disabled={busy() || !props.localBranches.length}
          onClick={() => void run(() => actions.push(props.workspaceId, branch() ?? props.localBranches[0], remote() ?? remotes()[0]))}
          data-testid="push"
        >
          Push
        </Button>
      </div>
    </Show>
  );
}
