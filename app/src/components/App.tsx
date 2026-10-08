import { For, Match, onSettled, Show, Switch } from "solid-js";
import { AppContext, conversationOf, type App as AppModel } from "../store";
import { ConversationView } from "./ConversationView";
import { ProposalsView } from "./ProposalsView";
import { SettingsView } from "./SettingsView";
import { Sidebar } from "./Sidebar";
import { SkillsView } from "./SkillsView";
import { Button, Empty, ErrorBox, Spinner } from "./ui";
import { WorkspaceView } from "./WorkspaceView";

export function App(props: { app: AppModel }) {
  return (
    <AppContext value={props.app}>
      <Shell app={props.app} />
    </AppContext>
  );
}

function Shell(props: { app: AppModel }) {
  const state = props.app.state;
  const actions = props.app.actions;
  onSettled(() => {
    void actions.init();
    return () => actions.dispose();
  });
  return (
    <div class="flex h-full flex-col">
      <Show
        when={!state.loading || state.info}
        fallback={
          <div class="flex h-full items-center justify-center">
            <Spinner label="Starting nucleus…" />
          </div>
        }
      >
        <Show
          when={state.info?.ready}
          fallback={
            <div class="mx-auto mt-24 flex max-w-lg flex-col gap-4 p-6" data-testid="init-error">
              <h1 class="text-lg font-semibold">nucleus could not start</h1>
              <ErrorBox message={state.info?.error ?? "Unknown error"} />
              <p class="text-sm text-zinc-400">
                nucleus runs every agent in a container. Start Podman (<code>systemctl --user start podman.socket</code>) or Docker, or set
                NUCLEUS_CONTAINER_SOCKET, then retry.
              </p>
              <div>
                <Button onClick={() => void actions.init()} disabled={state.loading} data-testid="retry-init">
                  Retry
                </Button>
              </div>
            </div>
          }
        >
          <div class="flex min-h-0 flex-1">
            <Sidebar />
            <main class="flex min-w-0 flex-1 flex-col">
              <Switch>
                <Match when={state.view === "proposals"}>
                  <ProposalsView />
                </Match>
                <Match when={state.view === "skills"}>
                  <SkillsView />
                </Match>
                <Match when={state.view === "settings"}>
                  <SettingsView />
                </Match>
                <Match when={state.view === "conversation" && conversationOf(state)}>
                  {(conv) => <ConversationView conversation={conv()} />}
                </Match>
                <Match when={state.workspaces.find((w) => w.id === state.selectedWorkspace)}>{(ws) => <WorkspaceView workspace={ws()} />}</Match>
                <Match when={true}>
                  <Empty title="No workspace yet">
                    <p>Add a git repository in the sidebar to get started.</p>
                  </Empty>
                </Match>
              </Switch>
            </main>
          </div>
        </Show>
      </Show>
      <Show when={state.progress}>
        {(p) => (
          <div class="fixed bottom-4 left-4 z-50 rounded-md border border-zinc-800 bg-zinc-900 px-3 py-2 shadow-lg" data-testid="progress">
            <Spinner label={p()} />
          </div>
        )}
      </Show>
      <div class="fixed right-4 bottom-4 z-50 flex w-96 flex-col gap-2" aria-live="polite">
        <For each={state.toasts}>
          {(t) => (
            <div
              data-testid={`toast-${t.kind}`}
              class={[
                "flex items-start gap-2 rounded-md border px-3 py-2 text-sm shadow-lg",
                {
                  "border-red-900 bg-red-950 text-red-100": t.kind === "error",
                  "border-emerald-900 bg-emerald-950 text-emerald-100": t.kind === "success",
                  "border-zinc-700 bg-zinc-900 text-zinc-100": t.kind === "info",
                },
              ]}
            >
              <span class="flex-1 break-words whitespace-pre-wrap">{t.text}</span>
              <button type="button" aria-label="Dismiss" class="opacity-60 hover:opacity-100" onClick={() => actions.dismissToast(t.id)}>
                ✕
              </button>
            </div>
          )}
        </For>
      </div>
    </div>
  );
}
