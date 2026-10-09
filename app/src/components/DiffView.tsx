import { createEffect, createMemo, createSignal, For, Show } from "solid-js";
import type { FileDiff } from "../api/types";
import { diffStats, parsePatch } from "../lib/diff";
import { Badge, Empty, ErrorBox } from "./ui";

export function DiffView(props: { files: FileDiff[]; emptyText?: string }) {
  const stats = createMemo(() => diffStats(props.files));
  return (
    <div class="flex flex-col gap-3" data-testid="diff-view">
      <Show when={props.files.length} fallback={<Empty title={props.emptyText ?? "No changes"} />}>
        <p class="text-xs text-zinc-500" data-testid="diff-summary">
          {props.files.length} {props.files.length === 1 ? "file" : "files"} changed, <span class="text-emerald-400">+{stats().additions}</span>{" "}
          <span class="text-red-400">−{stats().deletions}</span>
        </p>
        <For each={props.files}>{(f, i) => <FileBlock file={f} open={i() < 20} />}</For>
      </Show>
    </div>
  );
}

const statusTone = { added: "emerald", deleted: "red", modified: "sky", renamed: "amber", copied: "amber" } as const;

function FileBlock(props: { file: FileDiff; open: boolean }) {
  const [open, setOpen] = createSignal(props.open);
  const hunks = createMemo(() => parsePatch(props.file.patch));
  return (
    <section class="overflow-hidden rounded-md border border-zinc-800" data-testid={`diff-file-${props.file.path}`}>
      <button type="button" class="flex w-full items-center gap-2 bg-zinc-900 px-3 py-1.5 text-left text-sm" onClick={() => setOpen((v) => !v)} aria-expanded={open() ? "true" : "false"}>
        <span class="text-zinc-500">{open() ? "▾" : "▸"}</span>
        <Badge tone={statusTone[props.file.status]}>{props.file.status}</Badge>
        <span class="flex-1 truncate font-mono text-xs text-zinc-200">
          <Show when={props.file.old_path}>{(o) => <span class="text-zinc-500">{o()} → </span>}</Show>
          {props.file.path}
        </span>
        <span class="font-mono text-xs text-emerald-400">+{props.file.additions}</span>
        <span class="font-mono text-xs text-red-400">−{props.file.deletions}</span>
      </button>
      <Show when={open()}>
        <Show when={props.file.truncated}>
          <p class="px-3 py-1 text-xs text-amber-300" data-testid="truncated">
            {props.file.patch ? "Diff truncated: the change is too large to show in full." : "File too large to diff."}
          </p>
        </Show>
        <Show when={!props.file.binary} fallback={<p class="px-3 py-2 text-xs text-zinc-500">Binary file not shown</p>}>
          <table class="w-full border-collapse font-mono text-xs">
            <tbody>
              <For each={hunks()}>
                {(h) => (
                  <>
                    <tr class="bg-indigo-950/40 text-indigo-300">
                      <td colspan={3} class="px-3 py-0.5">
                        {h.header}
                      </td>
                    </tr>
                    <For each={h.lines}>
                      {(l) => (
                        <tr
                          data-kind={l.kind}
                          class={{ "bg-emerald-950/50": l.kind === "add", "bg-red-950/50": l.kind === "del", "text-zinc-500 italic": l.kind === "meta" }}
                        >
                          <td class="w-10 px-2 text-right text-zinc-600 select-none">{l.oldNo ?? ""}</td>
                          <td class="w-10 px-2 text-right text-zinc-600 select-none">{l.newNo ?? ""}</td>
                          <td class="px-2 whitespace-pre-wrap break-all">
                            <span class="select-none text-zinc-600">{l.kind === "add" ? "+" : l.kind === "del" ? "−" : " "}</span>
                            {l.text}
                          </td>
                        </tr>
                      )}
                    </For>
                  </>
                )}
              </For>
            </tbody>
          </table>
        </Show>
      </Show>
    </section>
  );
}

/**
 * Loads a diff through a streaming backend call and renders files as they arrive. Restarts
 * whenever `streamKey` changes; results of superseded streams are dropped.
 */
export function StreamedDiff(props: { load: (onFile: (f: FileDiff) => void) => Promise<number>; streamKey: unknown; emptyText?: string }) {
  const [files, setFiles] = createSignal<FileDiff[]>([]);
  const [loading, setLoading] = createSignal(true);
  const [error, setError] = createSignal<string | null>(null);
  let generation = 0;
  createEffect(
    () => props.streamKey,
    () => {
      const mine = ++generation;
      setFiles([]);
      setError(null);
      setLoading(true);
      const pending: FileDiff[] = [];
      let scheduled = false;
      // Batch files arriving in the same tick into one update.
      const flushPending = () => {
        scheduled = false;
        if (mine !== generation) return;
        const batch = pending.splice(0);
        setFiles((f) => [...f, ...batch]);
      };
      props
        .load((f) => {
          if (mine !== generation) return;
          pending.push(f);
          if (!scheduled) {
            scheduled = true;
            queueMicrotask(flushPending);
          }
        })
        .then(
          () => {
            if (mine === generation) {
              flushPending();
              setLoading(false);
            }
          },
          (e) => {
            if (mine === generation) {
              setError(typeof e === "string" ? e : e instanceof Error ? e.message : String(e));
              setLoading(false);
            }
          },
        );
    },
  );
  return (
    <div data-testid="streamed-diff" data-loading={loading() ? "true" : "false"}>
      <Show when={error()}>{(e) => <ErrorBox message={e()} />}</Show>
      <Show when={loading()}>
        <p class="mb-2 text-xs text-zinc-500" data-testid="diff-loading">
          Loading diff… {files().length} {files().length === 1 ? "file" : "files"} so far
        </p>
      </Show>
      <Show when={!loading() || files().length > 0}>
        <DiffView files={files()} emptyText={props.emptyText} />
      </Show>
    </div>
  );
}
