import { createSignal, For, onSettled, Show } from "solid-js";
import type { Conversation, EgressLog } from "../api/types";
import { errorMessage } from "../lib/format";
import { useApp } from "../store";
import { Badge, Button, ErrorBox } from "./ui";

const modeText = {
  isolated: "No network: the sandbox cannot reach anything.",
  proxied: "Egress goes through the proxy; only the hosts below are reachable.",
  open: "Full network access: nothing is filtered or logged.",
} as const;

/** What the sandbox's egress proxy allowed and denied. Refreshes every three seconds. */
export function NetworkView(props: { conversation: Conversation }) {
  const { backend } = useApp();
  const [log, setLog] = createSignal<EgressLog | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  const load = async () => {
    try {
      setLog(await backend.egressLog(props.conversation.id));
      setError(null);
    } catch (e) {
      setError(errorMessage(e));
    }
  };
  onSettled(() => {
    void load();
    const id = setInterval(() => void load(), 3000);
    return () => clearInterval(id);
  });
  const denied = () => log()?.entries.filter((e) => e.verdict === "deny").length ?? 0;
  return (
    <div class="flex-1 overflow-y-auto px-6 py-4" data-testid="network-view">
      <Show when={error()}>{(e) => <ErrorBox message={e()} />}</Show>
      <Show when={log()}>
        {(l) => (
          <div class="flex flex-col gap-4">
            <div class="flex items-center gap-2">
              <Badge tone={l().mode === "open" ? "amber" : "emerald"} testid="network-mode">
                {l().mode}
              </Badge>
              <span class="flex-1 text-sm text-zinc-400">{modeText[l().mode]}</span>
              <Button size="sm" variant="secondary" onClick={() => void load()} data-testid="refresh-network">
                Refresh
              </Button>
            </div>
            <Show when={l().allowed.length}>
              <p class="text-xs text-zinc-500" data-testid="allowed-hosts">
                Allowed: {l().allowed.join(", ")}
              </p>
            </Show>
            <Show when={denied() > 0}>
              <p class="text-sm text-amber-300" data-testid="denied-summary">
                {denied()} {denied() === 1 ? "request was" : "requests were"} blocked. Add the host to the workspace allowlist if the agent needs it.
              </p>
            </Show>
            <Show when={l().mode === "proxied"}>
              <ul class="rounded-md border border-zinc-800 font-mono text-xs">
                <For each={[...l().entries].reverse()} fallback={<li class="p-3 text-zinc-500">No requests yet.</li>}>
                  {(e) => (
                    <li class="flex gap-3 border-b border-zinc-900 px-3 py-1" data-testid="egress-row" data-verdict={e.verdict}>
                      <span class="text-zinc-600">{new Date(e.time).toLocaleTimeString()}</span>
                      <Badge tone={e.verdict === "allow" ? "emerald" : "red"}>{e.verdict}</Badge>
                      <span class="text-zinc-300">{e.target}</span>
                    </li>
                  )}
                </For>
              </ul>
            </Show>
          </div>
        )}
      </Show>
    </div>
  );
}
