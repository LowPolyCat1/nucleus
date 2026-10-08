import { createMemo, createSignal, Errored, For, Loading, Show } from "solid-js";
import type { LibraryKind, Proposal } from "../api/types";
import { relativeTime } from "../lib/format";
import { useApp } from "../store";
import { DiffView } from "./DiffView";
import { Badge, Button, Empty, ErrorBox, errorText, Spinner } from "./ui";

export const kindTone = { skills: "indigo", tools: "emerald", templates: "amber" } as const;
const kindLabel: Record<LibraryKind, string> = { skills: "skill", tools: "tool", templates: "template" };

/** Pending proposals from agents. Nothing reaches a library without approval here. */
export function ProposalsView() {
  const { state } = useApp();
  const [selected, setSelected] = createSignal<Proposal | null>(null);
  const current = createMemo(() => {
    const s = selected();
    return s && state.proposals.some((p) => p.id === s.id) ? s : (state.proposals[0] ?? null);
  });
  return (
    <div class="flex min-h-0 flex-1" data-testid="proposals-view">
      <aside class="w-80 shrink-0 overflow-y-auto border-r border-zinc-800">
        <h1 class="px-4 pt-4 pb-2 text-lg font-semibold">Proposals</h1>
        <p class="px-4 pb-3 text-xs text-zinc-500">Skills, tools and templates the agent wants to add. Review each one; approved changes can be reverted later.</p>
        <For each={state.proposals} fallback={<p class="px-4 text-sm text-zinc-500">Nothing to review.</p>}>
          {(p) => (
            <button
              type="button"
              data-testid={`proposal-${p.id}`}
              onClick={() => setSelected(p)}
              class={["flex w-full flex-col gap-1 border-b border-zinc-900 px-4 py-2 text-left", current()?.id === p.id ? "bg-zinc-800/70" : "hover:bg-zinc-900"]}
            >
              <span class="flex items-center gap-2">
                <Badge tone={kindTone[p.kind]}>{kindLabel[p.kind]}</Badge>
                <span class="truncate text-sm text-zinc-200">{p.title}</span>
              </span>
              <span class="text-xs text-zinc-600">{relativeTime(p.created)}</span>
            </button>
          )}
        </For>
      </aside>
      <section class="min-w-0 flex-1 overflow-y-auto p-6">
        <Show when={current()} fallback={<Empty title="No pending proposals" />}>
          {(p) => <ProposalDetail proposal={p()} />}
        </Show>
      </section>
    </div>
  );
}

function ProposalDetail(props: { proposal: Proposal }) {
  const { actions, backend } = useApp();
  const [busy, setBusy] = createSignal(false);
  const detail = createMemo(() => backend.proposal(props.proposal.kind, props.proposal.id));
  const act = async (f: () => Promise<unknown>) => {
    setBusy(true);
    await f();
    setBusy(false);
  };
  return (
    <div class="flex flex-col gap-4" data-testid="proposal-detail">
      <div class="flex items-start gap-3">
        <div class="min-w-0 flex-1">
          <h2 class="text-lg font-semibold text-zinc-100">{props.proposal.title}</h2>
          <Show when={props.proposal.source}>{(s) => <p class="text-xs text-zinc-500">From conversation {s()}</p>}</Show>
        </div>
        <Button variant="secondary" disabled={busy()} onClick={() => void act(() => actions.reject(props.proposal.kind, props.proposal.id))} data-testid="reject">
          Reject
        </Button>
        <Button disabled={busy()} onClick={() => void act(() => actions.approve(props.proposal.kind, props.proposal.id))} data-testid="approve">
          Approve
        </Button>
      </div>
      <Show when={props.proposal.rationale}>
        <p class="rounded-md border border-zinc-800 bg-zinc-900/50 p-3 text-sm whitespace-pre-wrap text-zinc-300" data-testid="rationale">
          {props.proposal.rationale}
        </p>
      </Show>
      <Errored fallback={(err) => <ErrorBox message={errorText(err())} />}>
        <Loading fallback={<Spinner label="Loading diff…" />}>
          <DiffView files={detail().diff} />
        </Loading>
      </Errored>
    </div>
  );
}
