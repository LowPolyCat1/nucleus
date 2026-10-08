# nucleus

A desktop agent and coding harness. Every agent conversation gets its own git branch,
worktree and sandbox container. Agents grow their own libraries of skills, tools and dependency
templates, and every addition waits for your approval.

Built with Tauri 2 and Rust; the UI is SolidJS 2 with Tailwind. Desktop only (Linux, macOS,
Windows).

## How it works

```
 UI (SolidJS) ── Tauri IPC / events ── app (AppCore) ── harness ─┬─ core      agent loop, LlmProvider, Claude CLI
                                                                 ├─ sandbox   Podman/Docker via bollard, egress proxy
                                                                 ├─ vcs       gix + git CLI fallback, branch strategy
                                                                 ├─ templates prebuilt dependency mounts
                                                                 ├─ skills    SKILL.md library, usage tracking
                                                                 ├─ tools     sandboxed tools, MCP server
                                                                 └─ promotion proposals and approval (shared)
```

| Crate | Responsibility | Main trait |
| --- | --- | --- |
| `core` | Agent loop, events, Claude CLI provider (stream-json parser) | `LlmProvider`, `ProcessLauncher` |
| `sandbox` | Container lifecycle, exec, mounts, caches, network policy | `SandboxBackend` |
| `vcs` | Branches, commits, worktrees, diffs, merges | `Vcs`, `BranchStrategy` |
| `promotion` | Git-backed libraries with proposal branches | |
| `templates` | Template manifests, builds, conflict-checked mount resolution | |
| `skills` | Skill storage, search by description, outcome tracking | |
| `tools` | Tool manifests, sandboxed execution, test-then-propose, MCP server | `Tool` |
| `harness` | Workspaces, conversation lifecycle, turns, outbox, orphan cleanup | |
| `app/src-tauri` | Tauri shell and commands | |

### Conversations

Starting a conversation creates `agent/<id>` from the base branch you pick, a worktree outside
your working copy, and a container with the worktree mounted at `/workspace`. The Claude CLI
runs inside the container (`claude -p --output-format stream-json --verbose
--include-partial-messages`), so its built-in tools execute in the sandbox; output streams to
the UI as events. After every turn the harness commits the worktree on the agent branch.
Reviewing work is a branch diff; merging goes into any local branch (in place when it is
checked out, without touching any working copy otherwise).

Deleting a conversation removes its branch, worktree and container. If the branch has commits
not reachable from any local or origin branch, you choose: merge, keep a copy under `local/`,
or discard. On startup the harness removes orphaned agent branches, worktrees and containers.

### Sandbox and network

One bollard implementation serves Podman (preferred, rootless with `keep-id`) and Docker.
Containers drop capabilities, run as your user, and get package caches as shared volumes.
Network policy per workspace, default `none`:

- `none`: the container sits on an internal network; an egress proxy sidecar only lets through
  the model API hosts the CLI needs.
- `allowlist`: the same proxy, plus your hosts (`*.example.com` for subdomains).
- `full`: the engine's default network.

### Templates

A template is a prebuilt dependency directory plus `template.toml`:

```toml
name = "python"
mount = { mode = "readonly" }            # or "overlay" (Podman) or { mode = "worktree", path = "node_modules" }
env = { VIRTUAL_ENV = "/deps/python/venv" }
path_env = { PATH = ["/deps/python/venv/bin"] }
[build]
lockfiles = ["requirements.txt"]          # hashed with the image id: change either and it rebuilds
command = "python3 -m venv venv && venv/bin/pip install -r /src/requirements.txt"
network = ["pypi.org", "files.pythonhosted.org"]
```

Templates are built inside the agent image at the path they are mounted at, never copied
into workspaces. Several templates combine in declared order; two templates setting the same
plain variable or claiming overlapping worktree paths is rejected when you save the workspace.

### Skills, tools and approvals

Skills (`<name>/SKILL.md`), tools (`<name>/tool.toml` plus scripts) and templates each live in
their own git repository. The agent proposes changes through MCP tools served from inside the
container (`propose_skill`, `propose_tool`, `propose_template`); proposals become
`proposal/<id>` branches and reach `main` only when you approve them. Tool proposals must pass
their own test in the sandbox first. Approved changes can be reverted. Skill usage and
outcomes are tracked and unreliable skills are flagged.

## Running

Requirements: Rust (stable), Node 22, Podman or Docker, and on Linux `libwebkit2gtk-4.1-dev`.

```sh
podman build -t localhost/nucleus-agent:latest -f images/agent/Containerfile images/agent
cd app && npm ci && npx tauri dev
```

Set `ANTHROPIC_API_KEY` or `CLAUDE_CODE_OAUTH_TOKEN` in Settings. The UI alone runs in any
browser with `npm run dev` on an in-memory mock backend.

## Testing

`scripts/test.sh` runs formatting, clippy, all Rust tests, the TypeScript typecheck, vitest
and Playwright; none of it needs a container engine or an API key. `scripts/test.sh engine`
adds the tests against a real engine. See `CLAUDE.md` for details.
