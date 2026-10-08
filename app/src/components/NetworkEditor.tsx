import { createSignal, Show } from "solid-js";
import type { NetworkPolicy } from "../api/types";
import { parseHosts } from "../lib/network";
import { inputClass } from "./ui";

/** Edits a network policy. Calls `onChange` with a valid policy, or `null` while hosts are invalid. */
export function NetworkEditor(props: { value: NetworkPolicy; onChange: (p: NetworkPolicy | null) => void; name: string }) {
  const [mode, setMode] = createSignal<NetworkPolicy["mode"]>(props.value.mode);
  const [text, setText] = createSignal(props.value.mode === "allowlist" ? props.value.hosts.join("\n") : "");
  const [invalid, setInvalid] = createSignal<string[]>([]);

  const emit = (m: NetworkPolicy["mode"], t: string) => {
    if (m !== "allowlist") {
      setInvalid([]);
      props.onChange({ mode: m });
      return;
    }
    const parsed = parseHosts(t);
    setInvalid(parsed.invalid);
    props.onChange(parsed.invalid.length ? null : { mode: "allowlist", hosts: parsed.hosts });
  };

  const option = (m: NetworkPolicy["mode"], label: string, hint: string) => (
    <label class="flex cursor-pointer items-start gap-2 text-sm">
      <input
        type="radio"
        name={props.name}
        value={m}
        checked={mode() === m}
        onChange={() => {
          setMode(m);
          emit(m, text());
        }}
        class="mt-1"
      />
      <span>
        <span class="text-zinc-200">{label}</span>
        <span class="block text-xs text-zinc-500">{hint}</span>
      </span>
    </label>
  );

  return (
    <div class="flex flex-col gap-2" data-testid={`network-${props.name}`}>
      {option("none", "No network", "Only the model API is reachable. Safest against a prompt-injected agent exfiltrating data.")}
      {option("allowlist", "Allowlist", "The model API plus these hosts, through the egress proxy. *.example.com matches subdomains.")}
      <Show when={mode() === "allowlist"}>
        <textarea
          class={[inputClass, "h-24 font-mono"]}
          aria-label="Allowed hosts"
          placeholder={"registry.npmjs.org\npypi.org"}
          value={text()}
          onInput={(e) => {
            setText(e.currentTarget.value);
            emit("allowlist", e.currentTarget.value);
          }}
        />
        <Show when={invalid().length}>
          <p class="text-xs text-red-300" data-testid="invalid-hosts">
            Invalid hosts: {invalid().join(", ")}
          </p>
        </Show>
      </Show>
      {option("full", "Full access", "Unrestricted network. Only for trusted tasks.")}
    </div>
  );
}
