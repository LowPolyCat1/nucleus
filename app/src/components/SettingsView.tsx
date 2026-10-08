import { createSignal, Show } from "solid-js";
import { SECRET_MASK } from "../api/backend";
import type { NetworkPolicy, Settings } from "../api/types";
import { useApp } from "../store";
import { NetworkEditor } from "./NetworkEditor";
import { Button, inputClass } from "./ui";

const AUTH_KEYS = ["ANTHROPIC_API_KEY", "CLAUDE_CODE_OAUTH_TOKEN"] as const;

export function SettingsView() {
  const { state } = useApp();
  return (
    <div class="flex-1 overflow-y-auto p-6" data-testid="settings-view">
      <h1 class="mb-4 text-lg font-semibold">Settings</h1>
      {/* Keyed: the form re-initialises from the saved settings after every save. */}
      <Show when={state.settings} keyed>
        {(s) => <SettingsForm settings={s} />}
      </Show>
    </div>
  );
}

function SettingsForm(props: { settings: Settings }) {
  const { state, actions } = useApp();
  const [image, setImage] = createSignal(props.settings.image);
  const [model, setModel] = createSignal(props.settings.model ?? "");
  const [secrets, setSecrets] = createSignal<Record<string, string>>({ ...props.settings.provider_env });
  const [network, setNetwork] = createSignal<NetworkPolicy | null>(props.settings.default_network);
  const [saving, setSaving] = createSignal(false);

  const save = async (e: SubmitEvent) => {
    e.preventDefault();
    const n = network();
    if (!n) return;
    setSaving(true);
    const provider_env = Object.fromEntries(Object.entries(secrets()).filter(([, v]) => v !== ""));
    await actions.saveSettings({ image: image().trim(), model: model().trim() || null, provider_env, default_network: n });
    setSaving(false);
  };

  return (
    <form class="flex max-w-2xl flex-col gap-6" onSubmit={save}>
      <section class="flex flex-col gap-3">
        <h2 class="text-sm font-semibold text-zinc-200">Claude</h2>
        <p class="text-xs text-zinc-500">The Claude CLI runs inside each sandbox with your own credentials. Set one of them. Stored values are never shown again.</p>
        {AUTH_KEYS.map((key) => (
          <label class="flex flex-col gap-1 text-xs text-zinc-400">
            {key}
            <input
              type="password"
              class={[inputClass, "font-mono"]}
              autocomplete="off"
              aria-label={key}
              placeholder={secrets()[key] === SECRET_MASK ? "stored" : "not set"}
              value={secrets()[key] === SECRET_MASK ? "" : (secrets()[key] ?? "")}
              onInput={(e) => {
                const v = e.currentTarget.value;
                setSecrets((s) => ({ ...s, [key]: v === "" && props.settings.provider_env[key] === SECRET_MASK ? SECRET_MASK : v }));
              }}
            />
            <Show when={props.settings.provider_env[key] === SECRET_MASK}>
              <button
                type="button"
                class="self-start text-[11px] text-red-300 hover:underline"
                onClick={() => setSecrets((s) => ({ ...s, [key]: "" }))}
                data-testid={`clear-${key}`}
              >
                {secrets()[key] === "" ? "will be removed on save" : "Remove stored value"}
              </button>
            </Show>
          </label>
        ))}
        <label class="flex flex-col gap-1 text-xs text-zinc-400">
          Model (optional)
          <input class={inputClass} value={model()} onInput={(e) => setModel(e.currentTarget.value)} placeholder="CLI default" aria-label="Model" />
        </label>
      </section>
      <section class="flex flex-col gap-3">
        <h2 class="text-sm font-semibold text-zinc-200">Sandbox</h2>
        <label class="flex flex-col gap-1 text-xs text-zinc-400">
          Agent image
          <input class={[inputClass, "font-mono"]} value={image()} onInput={(e) => setImage(e.currentTarget.value)} aria-label="Agent image" />
        </label>
        <div class="flex gap-2">
          <Button variant="secondary" size="sm" onClick={() => void actions.buildImage()} data-testid="build-image">
            Build agent image
          </Button>
          <Button variant="secondary" size="sm" onClick={() => void actions.cleanup()} data-testid="cleanup">
            Clean up orphaned sandboxes
          </Button>
        </div>
        <p class="text-xs text-zinc-500">
          Engine: {state.info?.engine ?? "unknown"} · data in <code>{state.info?.data_dir}</code>
        </p>
      </section>
      <section class="flex flex-col gap-3">
        <h2 class="text-sm font-semibold text-zinc-200">Default network policy for new workspaces</h2>
        <NetworkEditor name="default" value={props.settings.default_network} onChange={setNetwork} />
      </section>
      <div>
        <Button type="submit" disabled={saving() || !network() || !image().trim()} data-testid="save-settings">
          {saving() ? "Saving…" : "Save settings"}
        </Button>
      </div>
    </form>
  );
}
