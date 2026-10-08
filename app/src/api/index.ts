import type { Backend } from "./backend";
import { MockBackend, type MockOptions } from "./mock";
import { isTauri, tauriBackend } from "./tauri";

declare global {
  interface Window {
    __nucleusMock?: MockBackend;
  }
}

/**
 * The backend for this environment: Tauri IPC inside the app, otherwise the in-memory mock.
 * Query parameters configure the mock for manual and end-to-end testing:
 * `?delay=0`, `?seed=0`, `?initError=message`, `?engine=docker`.
 */
export function createBackend(): Backend {
  if (isTauri()) return tauriBackend();
  const q = new URLSearchParams(window.location.search);
  const options: MockOptions = {
    delayMs: q.has("delay") ? Number(q.get("delay")) : 30,
    seed: q.get("seed") !== "0",
    engine: (q.get("engine") as MockOptions["engine"]) ?? "podman",
    initError: q.get("initError"),
  };
  const mock = new MockBackend(options);
  window.__nucleusMock = mock;
  return mock;
}

export type { Backend };
export { MockBackend };
