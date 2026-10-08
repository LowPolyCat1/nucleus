# nucleus: notes for agents working on this repo

Desktop agent harness (Tauri 2 + Rust, SolidJS 2.0.0-rc.10 + Tailwind 4 UI). Desktop only, no
mobile targets. Read `README.md` for the architecture.

## Verify every change

```sh
scripts/test.sh            # fmt, clippy, all Rust tests, typecheck, vitest, playwright
scripts/test.sh rust       # Rust only
scripts/test.sh frontend   # tsc + vitest
scripts/test.sh e2e        # playwright, headless chromium, mock backend
scripts/test.sh engine     # needs a running Podman or Docker (NUCLEUS_ENGINE_TESTS=1)
```

None of the default stages need a container engine, an API key or a display:

- Rust tests above the sandbox use `nucleus_sandbox::fake::FakeBackend` (feature `fake`), which
  emulates containers by running processes on the host with container paths mapped through
  the bind mounts. The harness tests drive a fake `claude` script (see
  `crates/harness/tests/common/mod.rs`; its behaviour is switched with a `mode` file).
- The UI talks to Rust only through `app/src/api/backend.ts`. `MockBackend`
  (`app/src/api/mock.ts`) implements it in memory and is used by vitest, playwright and by
  `npm run dev` in a plain browser. Keep the mock's behaviour faithful to the harness when you
  change either side.
- Engine tests (`*/tests/engine*.rs`, `templates/tests/build.rs`, `tools/tests/tools.rs`,
  `harness/tests/engine_e2e.rs`) skip unless `NUCLEUS_ENGINE_TESTS=1`. They use
  `node:22-alpine` and replace the Claude CLI with a fake from a template, so no API key.

## Driving the UI

- `cd app && npm run dev` serves the UI on http://localhost:1420 with the mock backend.
  Query parameters: `?delay=0` (instant streaming), `?seed=0` (empty state),
  `?initError=msg` (engine failure screen), `?engine=docker`.
- In the browser `window.__nucleusMock` is the mock: `failNext(method, message)`,
  `setInitError(msg)`, `emit(event)`, `commitOnBranch(...)`, `calls`.
- Every interactive element has a `data-testid` or an accessible name; prefer those in tests.
- Real app: `cd app && npx tauri dev` (needs webkit2gtk-4.1 on Linux). Headless smoke run:
  `xvfb-run -a target/debug/nucleus-app` with `npm run dev` serving the UI.
  `NUCLEUS_DATA_DIR` overrides the data directory.

## Solid 2 rules that bite (see `app/node_modules/solid-js/CHEATSHEET.md`)

- `createEffect(compute, apply)` only; no writes in component bodies or memos; write in
  handlers, `onSettled` or effect apply.
- Props are values: `<X v={sig()} />`, never destructure props.
- DOM updates land after a microtask: in tests use `findBy*` or `flush()` before asserting.
- A signal set in a handler still reads its old value in that same handler; pass the new value
  along explicitly instead of re-reading it.
- `<select value>` does not stick when options render later; use `selected` on `<option>`.
- Store proxies cannot be `structuredClone`d.
- `tests/unit/solid-contract.test.tsx` pins the framework behaviour the UI relies on.

## Conventions

- Rust errors are `anyhow` with context; IPC errors are the full chain as a string.
- Any IPC shape change must be mirrored in `app/src/api/types.ts`.
- Secrets never go to the UI: `get_state` masks provider env (`settings.rs`).
- Do not write the character U+2014 (em dash) in code, comments or docs.
