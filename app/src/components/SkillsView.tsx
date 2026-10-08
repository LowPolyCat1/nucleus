import { createMemo, createSignal, Errored, For, Loading, Show } from "solid-js";
import type { LibraryKind } from "../api/types";
import { relativeTime, reliability, shortId } from "../lib/format";
import { useApp } from "../store";
import { kindTone } from "./ProposalsView";
import { Badge, Button, ErrorBox, errorText, Spinner } from "./ui";
import { Tab } from "./WorkspaceView";

export function SkillsView() {
  const { state, backend } = useApp();
  const [kind, setKind] = createSignal<LibraryKind>("skills");
  const skills = createMemo(async () => {
    void state.libraryVersion;
    return backend.skills();
  });
  return (
    <div class="flex-1 overflow-y-auto p-6" data-testid="skills-view">
      <h1 class="mb-1 text-lg font-semibold">Skills & tools</h1>
      <p class="mb-4 text-sm text-zinc-500">What the agent has learned. Skills with poor outcomes are flagged so they can be removed before they do harm.</p>
      <Errored fallback={(err) => <ErrorBox message={errorText(err())} />}>
        <Loading fallback={<Spinner />}>
          <ul class="mb-8 flex flex-col gap-2">
            <For each={skills()} fallback={<li class="text-sm text-zinc-500">No skills yet.</li>}>
              {(s) => {
                const flagged = () => s.stats.failures + s.stats.negative >= 3 && reliability(s.stats) < 0.5;
                return (
                  <li class={["rounded-md border p-3", flagged() ? "border-red-900/70 bg-red-950/20" : "border-zinc-800"]} data-testid={`skill-${s.name}`} data-flagged={flagged() ? "true" : "false"}>
                    <div class="flex items-center gap-2">
                      <span class="font-medium text-zinc-100">{s.name}</span>
                      <Show when={flagged()}>
                        <Badge tone="red">review: often fails</Badge>
                      </Show>
                      <span class="flex-1" />
                      <span class="text-xs text-zinc-500">
                        {s.stats.uses} uses · {Math.round(reliability(s.stats) * 100)}% reliable
                        <Show when={s.stats.last_used}>{(t) => <> · last {relativeTime(t())}</>}</Show>
                      </span>
                    </div>
                    <p class="mt-1 text-sm text-zinc-400">{s.description}</p>
                    <Show when={s.when_to_use}>{(w) => <p class="mt-1 text-xs text-zinc-500">Use when: {w()}</p>}</Show>
                  </li>
                );
              }}
            </For>
          </ul>
        </Loading>
      </Errored>
      <h2 class="mb-2 text-sm font-semibold text-zinc-200">Library history</h2>
      <div class="mb-3 flex gap-1 border-b border-zinc-800" role="tablist">
        <Tab active={kind() === "skills"} onClick={() => setKind("skills")} label="Skills" />
        <Tab active={kind() === "tools"} onClick={() => setKind("tools")} label="Tools" />
        <Tab active={kind() === "templates"} onClick={() => setKind("templates")} label="Templates" />
      </div>
      <History kind={kind()} />
    </div>
  );
}

function History(props: { kind: LibraryKind }) {
  const { state, actions, backend } = useApp();
  const history = createMemo(async () => {
    void state.libraryVersion;
    return backend.history(props.kind, 50);
  });
  return (
    <Errored fallback={(err) => <ErrorBox message={errorText(err())} />}>
      <Loading fallback={<Spinner />}>
        <ul class="flex flex-col" data-testid="library-history">
          <For each={history()} fallback={<li class="text-sm text-zinc-500">No changes yet.</li>}>
            {(c) => (
              <li class="flex items-center gap-3 border-b border-zinc-900 py-1.5 text-sm">
                <Badge tone={kindTone[props.kind]}>{shortId(c.id)}</Badge>
                <span class="flex-1 truncate text-zinc-300">{c.summary}</span>
                <span class="text-xs text-zinc-600">{relativeTime(c.time)}</span>
                <Show when={c.summary.startsWith("Approve")}>
                  <Button size="sm" variant="ghost" onClick={() => void actions.revert(props.kind, c.id)} data-testid={`revert-${c.id}`}>
                    Revert
                  </Button>
                </Show>
              </li>
            )}
          </For>
        </ul>
      </Loading>
    </Errored>
  );
}
