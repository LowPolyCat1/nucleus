import { createSignal, For, onSettled, Show } from "solid-js";
import type { LogEntry, LogLevel } from "../api/types";
import { errorMessage } from "../lib/format";
import { useApp } from "../store";
import { Badge, Button, ErrorBox, inputBase } from "./ui";

const tone = { error: "red", warn: "amber", info: "sky", debug: "zinc", trace: "zinc" } as const;

/** Recent harness logs. Refreshes every two seconds while open. */
export function LogsView() {
  const { backend } = useApp();
  const [level, setLevel] = createSignal<LogLevel>("info");
  const [entries, setEntries] = createSignal<LogEntry[]>([]);
  const [error, setError] = createSignal<string | null>(null);
  const [filter, setFilter] = createSignal("");

  // Takes the level explicitly: a signal set in the same handler still reads the old value.
  const load = async (lvl: LogLevel = level()) => {
    try {
      setEntries(await backend.recentLogs(lvl));
      setError(null);
    } catch (e) {
      setError(errorMessage(e));
    }
  };
  onSettled(() => {
    void load();
    const id = setInterval(() => void load(), 2000);
    return () => clearInterval(id);
  });
  const visible = () => {
    const f = filter().toLowerCase();
    const list = f ? entries().filter((e) => e.message.toLowerCase().includes(f) || e.target.toLowerCase().includes(f)) : entries();
    return [...list].reverse();
  };

  return (
    <div class="flex min-h-0 flex-1 flex-col p-6" data-testid="logs-view">
      <div class="mb-3 flex items-center gap-2">
        <h1 class="flex-1 text-lg font-semibold">Logs</h1>
        <input class={[inputBase, "w-56"]} placeholder="Filter" value={filter()} onInput={(e) => setFilter(e.currentTarget.value)} aria-label="Filter logs" />
        <select
          class={[inputBase, "w-28"]}
          aria-label="Minimum level"
          onChange={(e) => {
            const lvl = e.currentTarget.value as LogLevel;
            setLevel(lvl);
            void load(lvl);
          }}
        >
          <For each={["error", "warn", "info", "debug", "trace"] as LogLevel[]}>
            {(l) => (
              <option value={l} selected={l === level()}>
                {l}
              </option>
            )}
          </For>
        </select>
        <Button variant="secondary" size="sm" onClick={() => void load()} data-testid="refresh-logs">
          Refresh
        </Button>
      </div>
      <Show when={error()}>{(e) => <ErrorBox message={e()} />}</Show>
      <div class="flex-1 overflow-y-auto rounded-md border border-zinc-800 font-mono text-xs">
        <For each={visible()} fallback={<p class="p-4 text-zinc-500">No log entries.</p>}>
          {(e) => (
            <div class="flex gap-3 border-b border-zinc-900 px-3 py-1" data-testid="log-row" data-level={e.level}>
              <span class="shrink-0 text-zinc-600">{new Date(e.time).toLocaleTimeString()}</span>
              <Badge tone={tone[e.level]}>{e.level}</Badge>
              <span class="shrink-0 text-zinc-500">{e.target}</span>
              <span class="break-all whitespace-pre-wrap text-zinc-300">{e.message}</span>
            </div>
          )}
        </For>
      </div>
    </div>
  );
}
