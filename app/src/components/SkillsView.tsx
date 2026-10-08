import { createMemo, createSignal, Errored, For, Loading, Show } from "solid-js";
import type { LibraryKind, SkillSummary } from "../api/types";
import { errorMessage, relativeTime, reliability, shortId } from "../lib/format";
import { useApp } from "../store";
import { LibraryEditor, SKILL_SKELETON, TEMPLATE_SKELETON, type EditorKind } from "./LibraryEditor";
import { kindTone } from "./ProposalsView";
import { Badge, Button, ErrorBox, errorText, inputClass, Modal, Spinner } from "./ui";
import { Tab } from "./WorkspaceView";

export function isFlagged(s: SkillSummary): boolean {
  return s.stats.failures + s.stats.negative >= 3 && reliability(s.stats) < 0.5;
}

export function SkillsView() {
  const { state, actions, backend } = useApp();
  const [kind, setKind] = createSignal<LibraryKind>("skills");
  const [editor, setEditor] = createSignal<{ kind: EditorKind; initial: string; editing: boolean } | null>(null);
  const [removing, setRemoving] = createSignal<SkillSummary | null>(null);
  const skills = createMemo(async () => {
    void state.libraryVersion;
    return backend.skills();
  });
  const tools = createMemo(async () => {
    void state.libraryVersion;
    return backend.tools();
  });
  const templates = createMemo(async () => {
    void state.libraryVersion;
    return backend.availableTemplates();
  });
  const edit = async (k: EditorKind, name: string) => {
    try {
      const initial = k === "skill" ? await backend.skillSource(name) : await backend.templateSource(name);
      setEditor({ kind: k, initial, editing: true });
    } catch (e) {
      actions.toast("error", errorMessage(e));
    }
  };

  return (
    <div class="flex-1 overflow-y-auto p-6" data-testid="skills-view">
      <div class="mb-1 flex items-center gap-2">
        <h1 class="flex-1 text-lg font-semibold">Skills & tools</h1>
        <Button size="sm" variant="secondary" onClick={() => setEditor({ kind: "skill", initial: SKILL_SKELETON, editing: false })} data-testid="new-skill">
          New skill
        </Button>
        <Button size="sm" variant="secondary" onClick={() => setEditor({ kind: "template", initial: TEMPLATE_SKELETON, editing: false })} data-testid="new-template">
          New template
        </Button>
      </div>
      <p class="mb-4 text-sm text-zinc-500">What the agent has learned. Every change, yours or the agent's, goes through Proposals. Skills with poor outcomes are flagged.</p>

      <h2 class="mb-2 text-sm font-semibold text-zinc-200">Skills</h2>
      <Errored fallback={(err) => <ErrorBox message={errorText(err())} />}>
        <Loading fallback={<Spinner />}>
          <ul class="mb-8 flex flex-col gap-2">
            <For each={skills()} fallback={<li class="text-sm text-zinc-500">No skills yet.</li>}>
              {(s) => (
                <li class={["rounded-md border p-3", isFlagged(s) ? "border-red-900/70 bg-red-950/20" : "border-zinc-800"]} data-testid={`skill-${s.name}`} data-flagged={isFlagged(s) ? "true" : "false"}>
                  <div class="flex items-center gap-2">
                    <span class="font-medium text-zinc-100">{s.name}</span>
                    <Show when={isFlagged(s)}>
                      <Badge tone="red">review: often wrong</Badge>
                    </Show>
                    <span class="flex-1" />
                    <span class="text-xs text-zinc-500" data-testid="skill-stats">
                      {s.stats.uses} uses · {Math.round(reliability(s.stats) * 100)}% reliable
                      <Show when={s.stats.last_used}>{(t) => <> · last {relativeTime(t())}</>}</Show>
                    </span>
                    <Button size="sm" variant="ghost" onClick={() => void edit("skill", s.name)} data-testid={`edit-skill-${s.name}`}>
                      Edit
                    </Button>
                    <Button size="sm" variant="ghost" onClick={() => setRemoving(s)} data-testid={`remove-skill-${s.name}`}>
                      Propose removal
                    </Button>
                  </div>
                  <p class="mt-1 text-sm text-zinc-400">{s.description}</p>
                  <Show when={s.when_to_use}>{(w) => <p class="mt-1 text-xs text-zinc-500">Use when: {w()}</p>}</Show>
                </li>
              )}
            </For>
          </ul>
        </Loading>
      </Errored>

      <div class="mb-8 grid gap-6 lg:grid-cols-2">
        <section>
          <h2 class="mb-2 text-sm font-semibold text-zinc-200">Tools</h2>
          <Errored fallback={(err) => <ErrorBox message={errorText(err())} />}>
            <Loading fallback={<Spinner />}>
              <ul class="flex flex-col gap-1" data-testid="tools-list">
                <For each={tools()} fallback={<li class="text-sm text-zinc-500">No tools yet. The agent proposes tools it builds and tests.</li>}>
                  {(t) => (
                    <li class="rounded-md border border-zinc-800 px-3 py-2 text-sm" data-testid={`tool-${t.name}`}>
                      <span class="font-medium text-zinc-100">{t.name}</span>
                      <span class="ml-2 text-zinc-400">{t.description}</span>
                      <p class="mt-1 font-mono text-xs text-zinc-500">
                        {t.run} · timeout {t.timeout_secs}s
                      </p>
                    </li>
                  )}
                </For>
              </ul>
            </Loading>
          </Errored>
        </section>
        <section>
          <h2 class="mb-2 text-sm font-semibold text-zinc-200">Templates</h2>
          <Errored fallback={(err) => <ErrorBox message={errorText(err())} />}>
            <Loading fallback={<Spinner />}>
              <ul class="flex flex-col gap-1" data-testid="templates-list">
                <For each={templates()} fallback={<li class="text-sm text-zinc-500">No templates yet.</li>}>
                  {(t) => (
                    <li class="flex items-center gap-2 rounded-md border border-zinc-800 px-3 py-2 text-sm" data-testid={`library-template-${t.name}`}>
                      <span class="font-medium text-zinc-100">{t.name}</span>
                      <Badge>{t.mount.mode}</Badge>
                      <span class="flex-1 truncate text-zinc-400">{t.description}</span>
                      <Button size="sm" variant="ghost" onClick={() => void edit("template", t.name)} data-testid={`edit-template-${t.name}`}>
                        Edit
                      </Button>
                    </li>
                  )}
                </For>
              </ul>
            </Loading>
          </Errored>
        </section>
      </div>

      <h2 class="mb-2 text-sm font-semibold text-zinc-200">Library history</h2>
      <div class="mb-3 flex gap-1 border-b border-zinc-800" role="tablist">
        <Tab active={kind() === "skills"} onClick={() => setKind("skills")} label="Skills" />
        <Tab active={kind() === "tools"} onClick={() => setKind("tools")} label="Tools" />
        <Tab active={kind() === "templates"} onClick={() => setKind("templates")} label="Templates" />
      </div>
      <History kind={kind()} />

      <Show when={editor()}>{(e) => <LibraryEditor kind={e().kind} initial={e().initial} editing={e().editing} onClose={() => setEditor(null)} />}</Show>
      <Show when={removing()}>{(s) => <RemoveSkill skill={s()} onClose={() => setRemoving(null)} />}</Show>
    </div>
  );
}

function RemoveSkill(props: { skill: SkillSummary; onClose: () => void }) {
  const { actions } = useApp();
  const [rationale, setRationale] = createSignal(
    isFlagged(props.skill)
      ? `Unreliable: ${Math.round(reliability(props.skill.stats) * 100)}% over ${props.skill.stats.uses} uses (${props.skill.stats.failures} failed, ${props.skill.stats.negative} marked wrong).`
      : "",
  );
  const [busy, setBusy] = createSignal(false);
  const submit = async (e: SubmitEvent) => {
    e.preventDefault();
    setBusy(true);
    const ok = await actions.propose("skill-removal", props.skill.name, rationale().trim() || "No longer wanted");
    setBusy(false);
    if (ok) props.onClose();
  };
  return (
    <Modal title={`Remove skill ${props.skill.name}`} onClose={() => props.onClose()} testid="remove-skill-dialog">
      <form class="flex flex-col gap-3" onSubmit={submit}>
        <p class="text-sm text-zinc-400">This creates a removal proposal; the skill stays until you approve it.</p>
        <input class={inputClass} aria-label="Reason" value={rationale()} onInput={(e) => setRationale(e.currentTarget.value)} />
        <div class="flex justify-end gap-2">
          <Button variant="secondary" onClick={() => props.onClose()}>
            Cancel
          </Button>
          <Button variant="danger" type="submit" disabled={busy()} data-testid="confirm-remove-skill">
            Propose removal
          </Button>
        </div>
      </form>
    </Modal>
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
