import { createSignal, For, Match, Show, Switch } from "solid-js";
import type { CommitInfo, Conversation, DeleteMode } from "../api/types";
import { shortId } from "../lib/format";
import { useApp } from "../store";
import { Button, ErrorBox, inputClass, Modal } from "./ui";

type Phase = { step: "confirm" } | { step: "unmerged"; commits: CommitInfo[] } | { step: "conflicts"; paths: string[] };

/**
 * Deleting a conversation removes its branch, worktree and container. Work not reachable from
 * any local or origin branch needs explicit confirmation: merge it, keep a copy, or discard it.
 */
export function DeleteDialog(props: { conversation: Conversation; onClose: () => void }) {
  const { actions } = useApp();
  const [phase, setPhase] = createSignal<Phase>({ step: "confirm" });
  const [busy, setBusy] = createSignal(false);
  const [mergeTarget, setMergeTarget] = createSignal(props.conversation.base_branch);
  const [copyName, setCopyName] = createSignal(`local/${props.conversation.id}`);

  const run = async (mode: DeleteMode) => {
    setBusy(true);
    const out = await actions.deleteConversation(props.conversation.id, mode);
    setBusy(false);
    if (!out) return;
    switch (out.result) {
      case "deleted":
        props.onClose();
        break;
      case "needs_confirmation":
        setPhase({ step: "unmerged", commits: out.unmerged });
        break;
      case "merge_conflicts":
        setPhase({ step: "conflicts", paths: out.paths });
        break;
    }
  };

  return (
    <Modal title="Delete conversation" onClose={() => props.onClose()} testid="delete-dialog">
      <Switch>
        <Match when={phase().step === "confirm"}>
          <p class="mb-4 text-sm text-zinc-300">
            This deletes the branch <code class="text-indigo-300">{props.conversation.branch}</code>, its worktree and its container.
          </p>
          <div class="flex justify-end gap-2">
            <Button variant="secondary" onClick={() => props.onClose()}>
              Cancel
            </Button>
            <Button variant="danger" disabled={busy()} onClick={() => void run({ mode: "check" })} data-testid="confirm-delete">
              Delete
            </Button>
          </div>
        </Match>
        <Match when={phase().step === "unmerged" && (phase() as Extract<Phase, { step: "unmerged" }>)}>
          {(p) => (
            <div class="flex flex-col gap-4" data-testid="unmerged-warning">
              <p class="text-sm text-amber-200">
                {p().commits.length} {p().commits.length === 1 ? "commit is" : "commits are"} only on this agent branch and would be lost.
              </p>
              <ul class="max-h-40 overflow-y-auto rounded-md border border-zinc-800 p-2 text-xs">
                <For each={p().commits}>
                  {(c) => (
                    <li class="flex gap-2">
                      <span class="font-mono text-zinc-600">{shortId(c.id)}</span>
                      <span class="truncate">{c.summary}</span>
                    </li>
                  )}
                </For>
              </ul>
              <div class="flex items-end gap-2">
                <label class="flex flex-1 flex-col gap-1 text-xs text-zinc-400">
                  Merge into
                  <input class={inputClass} value={mergeTarget()} onInput={(e) => setMergeTarget(e.currentTarget.value)} aria-label="Merge into branch" />
                </label>
                <Button disabled={busy() || !mergeTarget().trim()} onClick={() => void run({ mode: "merge_into", branch: mergeTarget().trim() })} data-testid="merge-and-delete">
                  Merge, then delete
                </Button>
              </div>
              <div class="flex items-end gap-2">
                <label class="flex flex-1 flex-col gap-1 text-xs text-zinc-400">
                  Keep a copy as
                  <input class={inputClass} value={copyName()} onInput={(e) => setCopyName(e.currentTarget.value)} aria-label="Copy branch name" />
                </label>
                <Button variant="secondary" disabled={busy() || !copyName().trim()} onClick={() => void run({ mode: "keep_copy", branch: copyName().trim() })} data-testid="keep-copy">
                  Keep copy, then delete
                </Button>
              </div>
              <div class="flex justify-between border-t border-zinc-800 pt-4">
                <Button variant="secondary" onClick={() => props.onClose()}>
                  Cancel
                </Button>
                <Button variant="danger" disabled={busy()} onClick={() => void run({ mode: "discard" })} data-testid="discard">
                  Discard work and delete
                </Button>
              </div>
            </div>
          )}
        </Match>
        <Match when={phase().step === "conflicts" && (phase() as Extract<Phase, { step: "conflicts" }>)}>
          {(p) => (
            <div class="flex flex-col gap-3" data-testid="merge-conflicts">
              <ErrorBox message={`Merging into ${mergeTarget()} conflicts in:\n${p().paths.join("\n")}\n\nNothing was deleted.`} />
              <Show when={true}>
                <div class="flex justify-end gap-2">
                  <Button variant="secondary" onClick={() => setPhase({ step: "confirm" })}>
                    Back
                  </Button>
                  <Button variant="secondary" onClick={() => props.onClose()}>
                    Close
                  </Button>
                </div>
              </Show>
            </div>
          )}
        </Match>
      </Switch>
    </Modal>
  );
}
