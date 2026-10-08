import { render } from "@solidjs/testing-library";
import { MockBackend, type MockOptions } from "../../src/api/mock";
import { App } from "../../src/components/App";
import { createApp } from "../../src/store";

/** Render the whole app on a fresh mock backend. Streams are instant and toasts stay put. */
export function renderApp(options: MockOptions = {}) {
  const backend = new MockBackend({ delayMs: 0, ...options });
  const app = createApp(backend, { toastMs: 0 });
  const r = render(() => <App app={app} />);
  return { ...r, backend, app };
}

export async function withKey(backend: MockBackend) {
  const s = (await backend.state()).settings;
  await backend.updateSettings({ ...s, provider_env: { ANTHROPIC_API_KEY: "sk-test" } });
}
