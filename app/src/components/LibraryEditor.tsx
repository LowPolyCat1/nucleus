import { createSignal, Show } from "solid-js";
import { useApp } from "../store";
import { Button, inputClass, Modal } from "./ui";

export type EditorKind = "skill" | "template";

export const SKILL_SKELETON = `---
name: my-skill
description: One sentence on what this skill does
when_to_use: When the agent should load it
---

Step-by-step instructions for the agent.
`;

export const TEMPLATE_SKELETON = `name = "my-deps"
description = "What this template provides"
mount = { mode = "readonly" }
path_env = { PATH = ["/deps/my-deps/bin"] }

[build]
lockfiles = []
command = "mkdir -p bin"
network = []
`;

/** Write or edit a skill or template by hand. Submitting creates a proposal, nothing is applied directly. */
export function LibraryEditor(props: { kind: EditorKind; initial: string; editing: boolean; onClose: () => void }) {
  const { actions } = useApp();
  const [text, setText] = createSignal(props.initial);
  const [rationale, setRationale] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const submit = async (e: SubmitEvent) => {
    e.preventDefault();
    setBusy(true);
    const ok = await actions.propose(props.kind, text(), rationale().trim() || (props.editing ? "Edited by hand" : "Written by hand"));
    setBusy(false);
    if (ok) props.onClose();
  };
  return (
    <Modal title={`${props.editing ? "Edit" : "New"} ${props.kind}`} onClose={() => props.onClose()} testid="library-editor">
      <form class="flex flex-col gap-3" onSubmit={submit}>
        <textarea
          class={[inputClass, "h-72 font-mono text-xs"]}
          aria-label={props.kind === "skill" ? "SKILL.md" : "template.toml"}
          value={text()}
          onInput={(e) => setText(e.currentTarget.value)}
          spellcheck={false}
        />
        <input class={inputClass} placeholder="Why? (shown in review)" aria-label="Rationale" value={rationale()} onInput={(e) => setRationale(e.currentTarget.value)} />
        <Show when={props.kind === "template"}>
          <p class="text-xs text-zinc-500">The template is built in the agent image at its mount path when a workspace first uses it.</p>
        </Show>
        <div class="flex justify-end gap-2">
          <Button variant="secondary" onClick={() => props.onClose()}>
            Cancel
          </Button>
          <Button type="submit" disabled={busy() || !text().trim()} data-testid="submit-proposal">
            Propose
          </Button>
        </div>
      </form>
    </Modal>
  );
}
