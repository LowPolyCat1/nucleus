import { validateSettings } from "../lib/settings";
import { SECRET_MASK, type Backend } from "./backend";
import type {
  AgentEvent,
  AppInfo,
  BranchInfo,
  CleanupReport,
  CommitInfo,
  Conversation,
  DeleteOutcome,
  Engine,
  FileDiff,
  HarnessEvent,
  LogEntry,
  LogLevel,
  LibraryKind,
  MergeOutcome,
  NetworkPolicy,
  Proposal,
  Settings,
  SkillSummary,
  TemplateManifest,
  SkillStats,
  ToolManifest,
  TranscriptEntry,
  TurnSummary,
  Workspace,
} from "./types";

/**
 * In-memory backend with realistic behaviour: git-like history per workspace, streamed turns,
 * proposals with approval, unmerged-work checks on delete. Used in the browser (no Tauri), in
 * unit tests and in end-to-end tests. Exposed as `window.__nucleusMock` for test control.
 */
export interface MockOptions {
  /** Delay between streamed events in ms. 0 streams on microtasks. */
  delayMs?: number;
  /** Start with a demo workspace, conversation and proposals. */
  seed?: boolean;
  engine?: Engine | null;
  /** Make `init` fail with this message until cleared. */
  initError?: string | null;
}

interface MockCommit extends CommitInfo {
  files: Record<string, string>;
}

interface MockRepo {
  commits: Map<string, MockCommit>;
  branches: Map<string, string>; // name -> commit id
  head: string;
}

const id = (n = 12) => {
  let s = "";
  while (s.length < n) s += crypto.randomUUID().replace(/-/g, "");
  return s.slice(0, n);
};

/** Deep copy through JSON, like IPC serialisation. Works on store proxies, unlike structuredClone. */
const clone = <T,>(v: T): T => JSON.parse(JSON.stringify(v));

const now = () => Math.floor(Date.now() / 1000);

function patchFor(oldText: string | undefined, newText: string | undefined): { patch: string; additions: number; deletions: number } {
  const a = oldText === undefined ? [] : oldText.split("\n").filter((_, i, arr) => i < arr.length - 1 || arr[i] !== "");
  const b = newText === undefined ? [] : newText.split("\n").filter((_, i, arr) => i < arr.length - 1 || arr[i] !== "");
  let start = 0;
  while (start < a.length && start < b.length && a[start] === b[start]) start++;
  let endA = a.length;
  let endB = b.length;
  while (endA > start && endB > start && a[endA - 1] === b[endB - 1]) {
    endA--;
    endB--;
  }
  const del = a.slice(start, endA);
  const add = b.slice(start, endB);
  const ctxBefore = a.slice(Math.max(0, start - 3), start);
  const ctxAfter = a.slice(endA, endA + 3);
  const oldStart = a.length ? Math.max(1, start - ctxBefore.length + 1) : 0;
  const newStart = b.length ? Math.max(1, start - ctxBefore.length + 1) : 0;
  const oldLen = ctxBefore.length + del.length + ctxAfter.length;
  const newLen = ctxBefore.length + add.length + ctxAfter.length;
  const lines = [
    `@@ -${oldStart},${oldLen} +${newStart},${newLen} @@`,
    ...ctxBefore.map((l) => ` ${l}`),
    ...del.map((l) => `-${l}`),
    ...add.map((l) => `+${l}`),
    ...ctxAfter.map((l) => ` ${l}`),
  ];
  return { patch: lines.join("\n") + "\n", additions: add.length, deletions: del.length };
}

const TEMPLATES: TemplateManifest[] = [
  {
    name: "node",
    description: "pnpm node_modules for the workspace",
    mount: { mode: "worktree", path: "node_modules" },
    env: {},
    path_env: { PATH: ["/workspace/node_modules/.bin"] },
    build: { image: null, lockfiles: ["package.json", "pnpm-lock.yaml"], command: "pnpm install --frozen-lockfile", workdir: null, network: ["registry.npmjs.org"] },
  },
  {
    name: "python",
    description: "Python virtualenv with requirements.txt",
    mount: { mode: "readonly" },
    env: { VIRTUAL_ENV: "/deps/python/venv" },
    path_env: { PATH: ["/deps/python/venv/bin"] },
    build: { image: null, lockfiles: ["requirements.txt"], command: "python3 -m venv venv && venv/bin/pip install -r /src/requirements.txt", workdir: null, network: ["pypi.org", "files.pythonhosted.org"] },
  },
  {
    name: "python-alt",
    description: "Conflicts with python (sets VIRTUAL_ENV too)",
    mount: { mode: "readonly" },
    env: { VIRTUAL_ENV: "/deps/python-alt" },
    path_env: {},
    build: { image: null, lockfiles: [], command: "true", workdir: null, network: [] },
  },
];

/** Serialise a manifest as template.toml (the subset the mock needs). */
export function templateToToml(t: TemplateManifest): string {
  const inline = (o: Record<string, string | string[]>) =>
    `{ ${Object.entries(o)
      .map(([k, v]) => `${k} = ${Array.isArray(v) ? `[${v.map((x) => JSON.stringify(x)).join(", ")}]` : JSON.stringify(v)}`)
      .join(", ")} }`;
  const mount = t.mount.mode === "worktree" ? `{ mode = "worktree", path = ${JSON.stringify(t.mount.path)} }` : `{ mode = "${t.mount.mode}" }`;
  const lines = [`name = ${JSON.stringify(t.name)}`, `description = ${JSON.stringify(t.description)}`, `mount = ${mount}`];
  if (Object.keys(t.env).length) lines.push(`env = ${inline(t.env)}`);
  if (Object.keys(t.path_env).length) lines.push(`path_env = ${inline(t.path_env)}`);
  lines.push("[build]");
  if (t.build.lockfiles.length) lines.push(`lockfiles = [${t.build.lockfiles.map((l) => JSON.stringify(l)).join(", ")}]`);
  lines.push(`command = ${JSON.stringify(t.build.command)}`);
  if (t.build.network.length) lines.push(`network = [${t.build.network.map((l) => JSON.stringify(l)).join(", ")}]`);
  return lines.join("\n") + "\n";
}

/** Parse the template.toml subset written by hand in the UI. Throws a message like the Rust side. */
export function parseTemplateToml(text: string): TemplateManifest {
  const str = (key: string) => new RegExp(`^${key}\\s*=\\s*"([^"]*)"`, "m").exec(text)?.[1];
  const name = str("name");
  if (!name || !/^[A-Za-z0-9_-]+$/.test(name)) throw `template name ${JSON.stringify(name ?? "")} must be non-empty and use only letters, digits, '-' and '_'`;
  const mode = /mount\s*=\s*\{[^}]*mode\s*=\s*"(readonly|overlay|worktree)"/.exec(text)?.[1];
  if (!mode) throw "TOML parse error: missing field `mount`";
  const path = /mount\s*=\s*\{[^}]*path\s*=\s*"([^"]*)"/.exec(text)?.[1];
  if (mode === "worktree" && (!path || path.startsWith("/") || path.includes(".."))) throw `worktree mount path ${JSON.stringify(path ?? "")} must be a plain relative path`;
  const command = /^command\s*=\s*"([^"]*)"/m.exec(text)?.[1];
  if (!/^\[build\]/m.test(text) || command === undefined) throw "TOML parse error: missing field `build`";
  const table = (key: string): Record<string, string> => {
    const body = new RegExp(`^${key}\\s*=\\s*\\{([^}]*)\\}`, "m").exec(text)?.[1] ?? "";
    return Object.fromEntries([...body.matchAll(/([A-Za-z_][A-Za-z0-9_]*)\s*=\s*"([^"]*)"/g)].map((m) => [m[1], m[2]]));
  };
  return {
    name,
    description: str("description") ?? "",
    mount: mode === "worktree" ? { mode, path: path! } : { mode: mode as "readonly" | "overlay" },
    env: table("env"),
    path_env: {},
    build: { image: null, lockfiles: [], command, workdir: null, network: [] },
  };
}

interface PendingProposal extends Proposal {
  files: Record<string, string | null>;
}

export class MockBackend implements Backend {
  readonly options: Required<MockOptions>;
  private listeners = new Set<(e: HarnessEvent) => void>();
  private failures = new Map<string, string>();
  settings: Settings = {
    image: "localhost/nucleus-agent:latest",
    model: null,
    provider_env: {},
    default_network: { mode: "none" },
    permission_mode: "bypassPermissions",
    limits: { memory_mb: 8192, cpus: null, pids: 4096 },
  };
  private secrets: Record<string, string> = {};
  workspaces: Workspace[] = [];
  conversations: Conversation[] = [];
  private repos = new Map<string, MockRepo>();
  private transcripts = new Map<string, TranscriptEntry[]>();
  private libraries: Record<LibraryKind, { files: Record<string, string>; history: MockCommit[]; pending: PendingProposal[] }> = {
    skills: { files: {}, history: [], pending: [] },
    tools: { files: {}, history: [], pending: [] },
    templates: { files: {}, history: [], pending: [] },
  };
  private running = new Map<string, { cancelled: boolean }>();
  private builtTemplates = new Set<string>();
  logs: LogEntry[] = [];
  skillStats = new Map<string, SkillStats>();
  /** Every call, for assertions: `[method, args]`. */
  calls: [string, unknown[]][] = [];

  constructor(options: MockOptions = {}) {
    this.options = { delayMs: options.delayMs ?? 30, seed: options.seed ?? true, engine: options.engine === undefined ? "podman" : options.engine, initError: options.initError ?? null };
    // The template library always has a few entries; workspaces and conversations are demo data.
    this.libraries.templates.files = Object.fromEntries(TEMPLATES.map((t) => [`${t.name}/template.toml`, templateToToml(t)]));
    if (this.options.seed) this.seed();
  }

  // ---- test controls ----------------------------------------------------------------

  /** Make the next call of `method` reject with `message`. */
  failNext(method: keyof Backend, message: string) {
    this.failures.set(method, message);
  }

  setInitError(message: string | null) {
    this.options.initError = message;
  }

  emit(event: HarnessEvent) {
    for (const l of this.listeners) l(event);
  }

  /** Add a commit to a workspace branch as if the user committed outside the app. */
  commitOnBranch(workspaceId: string, branch: string, file: string, content: string, message = `Edit ${file}`) {
    const repo = this.repo(workspaceId);
    const parent = this.tip(repo, branch);
    this.commit(repo, branch, { ...repo.commits.get(parent)!.files, [file]: content }, message, [parent]);
  }

  // ---- internals --------------------------------------------------------------------

  log(level: LogLevel, message: string, target = "nucleus_harness") {
    this.logs.push({ time: Date.now(), level, target, message });
    if (this.logs.length > 2000) this.logs.shift();
  }

  private async guard<T>(method: string, args: unknown[], f: () => T | Promise<T>): Promise<T> {
    this.calls.push([method, args]);
    if (method !== "recentLogs") this.log("debug", `command ${method}`, "nucleus_app");
    await Promise.resolve();
    const fail = this.failures.get(method);
    if (fail !== undefined) {
      this.failures.delete(method);
      this.log("warn", `${method} failed: ${fail}`, "nucleus_app");
      throw fail;
    }
    try {
      return await f();
    } catch (e) {
      this.log("warn", `${method} failed: ${typeof e === "string" ? e : String(e)}`, "nucleus_app");
      throw e;
    }
  }

  private sleep() {
    return new Promise<void>((r) => (this.options.delayMs ? setTimeout(r, this.options.delayMs) : queueMicrotask(r)));
  }

  private seed() {
    const ws = this.addWorkspaceSync("/home/user/projects/demo", "demo");
    const repo = this.repo(ws.id);
    const main = this.tip(repo, "main");
    this.commit(repo, "main", { ...repo.commits.get(main)!.files, "src/lib.rs": "pub fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n" }, "Add add()", [main]);
    repo.branches.set("origin/main", this.tip(repo, "main"));
    this.commit(repo, "local/feature", { ...repo.commits.get(this.tip(repo, "main"))!.files, "docs/feature.md": "# Feature\n" }, "Draft feature docs", [this.tip(repo, "main")]);
    const conv = this.createConversationSync(ws.id, "main", "Add tests for add()");
    const crepo = this.repo(ws.id);
    const base = this.tip(crepo, conv.branch);
    this.commit(
      crepo,
      conv.branch,
      { ...crepo.commits.get(base)!.files, "src/lib.rs": "pub fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n\n#[test]\nfn adds() {\n    assert_eq!(add(2, 2), 4);\n}\n" },
      "Agent turn: Add tests for add()",
      [base],
    );
    this.transcripts.set(conv.id, [
      { role: "user", text: "Add tests for add()" },
      { role: "tool_use", id: "tu1", name: "Bash", input: { command: "cargo test" } },
      { role: "tool_result", tool_use_id: "tu1", content: "test result: ok. 1 passed", is_error: false },
      { role: "assistant", text: "I added a unit test for `add` and it passes." },
    ]);
    this.addPending("skills", "Add skill cargo-tests", "Running `cargo test` before finishing caught a bug.", conv.id, {
      "cargo-tests/SKILL.md": "---\nname: cargo-tests\ndescription: Run Rust tests with cargo\nwhen_to_use: Before finishing a Rust change\n---\n\nRun `cargo test` and fix failures.\n",
    });
    this.addPending("tools", "Add tool count-lines", "Counts lines per file.\n\nTest passed in the sandbox:\nok", conv.id, {
      "count-lines/tool.toml": 'name = "count-lines"\ndescription = "Count lines"\nrun = "wc -l"\ntest = "true"\n',
    });
    this.libraries.tools.files["word-count/tool.toml"] = 'name = "word-count"\ndescription = "Count words in a file"\nrun = "wc -w"\ntest = "echo a b | wc -w | grep -q 2"\ntimeout_secs = 30\n';
    this.skillStats.set("rust-style", { uses: 5, successes: 1, failures: 3, negative: 1, last_used: now() - 3600 });
    this.libraries.skills.files["rust-style/SKILL.md"] = "---\nname: rust-style\ndescription: House Rust style\n---\n";
    this.libraries.skills.history.push(this.libCommit("Add skill rust-style"));
  }

  private templateManifests(): TemplateManifest[] {
    return Object.entries(this.libraries.templates.files)
      .filter(([p]) => p.endsWith("/template.toml"))
      .map(([, text]) => parseTemplateToml(text))
      .sort((a, b) => a.name.localeCompare(b.name));
  }

  private libCommit(summary: string): MockCommit {
    return { id: id(40), summary, message: summary, author_name: "nucleus", author_email: "nucleus@localhost", time: now(), parents: [], files: {} };
  }

  private addPending(kind: LibraryKind, title: string, rationale: string, source: string | null, files: Record<string, string | null>) {
    const p: PendingProposal = { id: id(12), kind, title, rationale, source, commit: id(40), created: now(), files };
    this.libraries[kind].pending.unshift(p);
    return p;
  }

  private repo(workspaceId: string): MockRepo {
    const r = this.repos.get(workspaceId);
    if (!r) throw `no workspace ${workspaceId}`;
    return r;
  }

  private tip(repo: MockRepo, branch: string): string {
    const t = repo.branches.get(branch);
    if (!t) throw `cannot resolve ${branch}`;
    return t;
  }

  private resolve(repo: MockRepo, rev: string): string {
    const name = rev.replace(/^refs\/heads\//, "");
    return repo.branches.get(name) ?? (repo.commits.has(rev) ? rev : this.tip(repo, name));
  }

  private commit(repo: MockRepo, branch: string, files: Record<string, string>, message: string, parents: string[]): string {
    const c: MockCommit = { id: id(40), summary: message.split("\n")[0], message, author_name: "nucleus agent", author_email: "agent@nucleus.localhost", time: now() + repo.commits.size, parents, files };
    repo.commits.set(c.id, c);
    repo.branches.set(branch, c.id);
    return c.id;
  }

  private ancestors(repo: MockRepo, tip: string): Set<string> {
    const seen = new Set<string>();
    const stack = [tip];
    while (stack.length) {
      const c = stack.pop()!;
      if (seen.has(c) || !repo.commits.has(c)) continue;
      seen.add(c);
      stack.push(...repo.commits.get(c)!.parents);
    }
    return seen;
  }

  private addWorkspaceSync(path: string, name: string | null): Workspace {
    const trimmed = path.trim().replace(/\/+$/, "");
    if (!trimmed.startsWith("/")) throw `no git repository at ${path}`;
    if (this.workspaces.some((w) => w.repo === trimmed)) throw `${trimmed} is already a workspace`;
    const ws: Workspace = { id: id(12), name: name?.trim() || trimmed.split("/").pop() || trimmed, repo: trimmed, templates: [], network: clone(this.settings.default_network), template_builds: {} };
    const repo: MockRepo = { commits: new Map(), branches: new Map(), head: "main" };
    this.commit(repo, "main", { "README.md": `# ${ws.name}\n` }, "Initial commit", []);
    this.repos.set(ws.id, repo);
    this.workspaces.push(ws);
    return ws;
  }

  private createConversationSync(workspaceId: string, base: string, title: string): Conversation {
    const ws = this.workspaces.find((w) => w.id === workspaceId);
    if (!ws) throw `no workspace ${workspaceId}`;
    const repo = this.repo(workspaceId);
    const start = this.tip(repo, base);
    const cid = id(12);
    const conv: Conversation = {
      id: cid,
      workspace_id: workspaceId,
      title: title.trim() || "New conversation",
      base_branch: base,
      branch: `agent/${cid}`,
      worktree: `/data/worktrees/${cid}`,
      container: `nucleus-${cid}`,
      session_id: null,
      created: now(),
      status: "idle",
      last_turn_skills: [],
    };
    repo.branches.set(conv.branch, start);
    this.conversations.push(conv);
    this.transcripts.set(cid, []);
    return conv;
  }

  private conv(cid: string): Conversation {
    const c = this.conversations.find((c) => c.id === cid);
    if (!c) throw `no conversation ${cid}`;
    return c;
  }

  private diffCommits(repo: MockRepo, from: string, to: string): FileDiff[] {
    const a = repo.commits.get(this.resolve(repo, from))!.files;
    const b = repo.commits.get(this.resolve(repo, to))!.files;
    const paths = [...new Set([...Object.keys(a), ...Object.keys(b)])].sort();
    const out: FileDiff[] = [];
    for (const p of paths) {
      if (a[p] === b[p]) continue;
      const { patch, additions, deletions } = patchFor(a[p], b[p]);
      out.push({ path: p, old_path: null, status: a[p] === undefined ? "added" : b[p] === undefined ? "deleted" : "modified", binary: false, additions, deletions, patch });
    }
    return out;
  }

  private mergeBase(repo: MockRepo, a: string, b: string): string | null {
    const anc = this.ancestors(repo, a);
    // Newest common ancestor by time.
    let best: MockCommit | null = null;
    for (const c of this.ancestors(repo, b)) {
      if (anc.has(c)) {
        const commit = repo.commits.get(c)!;
        if (!best || commit.time > best.time) best = commit;
      }
    }
    return best?.id ?? null;
  }

  private unmerged(cid: string): CommitInfo[] {
    const c = this.conv(cid);
    const repo = this.repo(c.workspace_id);
    const hidden = new Set<string>();
    for (const [name, tip] of repo.branches) if (!name.startsWith("agent/")) for (const a of this.ancestors(repo, tip)) hidden.add(a);
    return [...this.ancestors(repo, this.tip(repo, c.branch))]
      .filter((x) => !hidden.has(x))
      .map((x) => strip(repo.commits.get(x)!))
      .sort((a, b) => b.time - a.time);
  }

  private mergeSync(cid: string, into: string): MergeOutcome {
    const c = this.conv(cid);
    if (into.startsWith("agent/")) throw "merge into a local branch, not another agent branch";
    const repo = this.repo(c.workspace_id);
    const target = this.tip(repo, into);
    const from = this.tip(repo, c.branch);
    if (this.ancestors(repo, target).has(from)) return { kind: "up_to_date" };
    if (this.ancestors(repo, from).has(target)) {
      repo.branches.set(into, from);
      return { kind: "fast_forward", commit: from };
    }
    const base = this.mergeBase(repo, target, from);
    const bf = base ? repo.commits.get(base)!.files : {};
    const tf = repo.commits.get(target)!.files;
    const ff = repo.commits.get(from)!.files;
    const merged: Record<string, string> = { ...tf };
    const conflicts: string[] = [];
    for (const p of new Set([...Object.keys(tf), ...Object.keys(ff)])) {
      const ours = tf[p] !== bf[p];
      const theirs = ff[p] !== bf[p];
      if (theirs && !ours) {
        if (ff[p] === undefined) delete merged[p];
        else merged[p] = ff[p];
      } else if (ours && theirs && tf[p] !== ff[p]) conflicts.push(p);
    }
    if (conflicts.length) return { kind: "conflicts", paths: conflicts.sort() };
    const commit = this.commit(repo, into, merged, `Merge ${c.branch}: ${c.title}`, [target, from]);
    return { kind: "merged", commit };
  }

  private deleteSync(cid: string) {
    const c = this.conv(cid);
    this.repo(c.workspace_id).branches.delete(c.branch);
    this.conversations = this.conversations.filter((x) => x.id !== cid);
    this.transcripts.delete(cid);
  }

  private setStatus(cid: string, status: Conversation["status"]) {
    const c = this.conversations.find((x) => x.id === cid);
    if (!c) return;
    c.status = status;
    this.emit({ type: "status", conversation_id: cid, status });
  }

  // ---- Backend ----------------------------------------------------------------------

  init(): Promise<AppInfo> {
    return this.guard("init", [], () => {
      if (this.options.initError) return { ready: false, engine: null, data_dir: "/mock", error: this.options.initError };
      return { ready: true, engine: this.options.engine, data_dir: "/mock/data", error: null };
    });
  }

  state() {
    return this.guard("state", [], () => ({
      settings: this.maskedSettings(),
      workspaces: clone(this.workspaces),
      conversations: clone(this.conversations),
    }));
  }

  private maskedSettings(): Settings {
    return { ...clone(this.settings), provider_env: Object.fromEntries(Object.keys(this.secrets).map((k) => [k, SECRET_MASK])) };
  }

  updateSettings(settings: Settings) {
    return this.guard("updateSettings", [settings], () => {
      const invalid = validateSettings(settings);
      if (invalid) throw invalid;
      const next: Record<string, string> = {};
      for (const [k, v] of Object.entries(settings.provider_env)) {
        if (!k.trim()) continue;
        if (v === SECRET_MASK) {
          if (this.secrets[k] !== undefined) next[k] = this.secrets[k];
        } else if (v !== "") next[k] = v;
      }
      this.secrets = next;
      this.settings = { ...clone(settings), provider_env: {} };
    });
  }

  pickDirectory() {
    return this.guard("pickDirectory", [], () => null);
  }

  addWorkspace(path: string, name: string | null) {
    return this.guard("addWorkspace", [path, name], () => clone(this.addWorkspaceSync(path, name)));
  }

  removeWorkspace(wid: string) {
    return this.guard("removeWorkspace", [wid], () => {
      if (this.conversations.some((c) => c.workspace_id === wid)) throw "delete the workspace's conversations first";
      this.workspaces = this.workspaces.filter((w) => w.id !== wid);
      this.repos.delete(wid);
    });
  }

  configureWorkspace(wid: string, templates: string[], network: NetworkPolicy) {
    return this.guard("configureWorkspace", [wid, templates, network], () => {
      const ws = this.workspaces.find((w) => w.id === wid);
      if (!ws) throw `no workspace ${wid}`;
      const seen = new Map<string, string>();
      for (const name of templates) {
        const t = this.templateManifests().find((t) => t.name === name);
        if (!t) throw `reading templates/${name}/template.toml: not found`;
        for (const k of Object.keys(t.env)) {
          if (seen.has(k)) throw `templates ${seen.get(k)} and ${name} both set ${k}`;
          seen.set(k, name);
        }
      }
      if (new Set(templates).size !== templates.length) throw "a template is listed twice";
      ws.templates = [...templates];
      ws.network = clone(network);
      return clone(ws);
    });
  }

  availableTemplates() {
    return this.guard("availableTemplates", [], () => this.templateManifests());
  }

  templateStatus(wid: string) {
    return this.guard("templateStatus", [wid], () => {
      const ws = this.workspaces.find((w) => w.id === wid);
      if (!ws) throw `no workspace ${wid}`;
      return ws.templates.map((name) => {
        const t = this.templateManifests().find((t) => t.name === name);
        return { name, description: t?.description ?? "", fresh: this.builtTemplates.has(`${wid}/${name}`), identity: `${name}-0123456789ab`, error: null };
      });
    });
  }

  buildTemplates(wid: string) {
    return this.guard("buildTemplates", [wid], async () => {
      const ws = this.workspaces.find((w) => w.id === wid);
      if (!ws) throw `no workspace ${wid}`;
      for (const name of ws.templates) {
        this.emit({ type: "progress", message: `Preparing template ${name}` });
        await this.sleep();
        this.builtTemplates.add(`${wid}/${name}`);
        this.emit({ type: "progress", message: `Built template ${name} (${name}-0123456789ab)` });
      }
    });
  }

  branches(wid: string) {
    return this.guard("branches", [wid], () => {
      const repo = this.repo(wid);
      return [...repo.branches.entries()]
        .map(([name, target]): BranchInfo => {
          const kind = name.startsWith("agent/") ? "agent" : name.startsWith("origin/") ? "origin" : "local";
          return { name, target, kind, full_ref: kind === "origin" ? `refs/remotes/${name}` : `refs/heads/${name}`, is_head: name === repo.head };
        })
        .sort((a, b) => a.name.localeCompare(b.name));
    });
  }

  graph(wid: string, limit: number) {
    return this.guard("graph", [wid, limit], () => {
      const repo = this.repo(wid);
      const all = new Set<string>();
      for (const tip of repo.branches.values()) for (const c of this.ancestors(repo, tip)) all.add(c);
      return [...all].map((c) => strip(repo.commits.get(c)!)).sort((a, b) => b.time - a.time).slice(0, limit);
    });
  }

  diff(wid: string, from: string, to: string) {
    return this.guard("diff", [wid, from, to], () => this.diffCommits(this.repo(wid), from, to));
  }

  createConversation(wid: string, base: string, title: string) {
    return this.guard("createConversation", [wid, base, title], async () => {
      const ws = this.workspaces.find((w) => w.id === wid);
      if (!ws) throw `no workspace ${wid}`;
      for (const name of ws.templates) {
        if (!this.builtTemplates.has(`${wid}/${name}`)) {
          this.emit({ type: "progress", message: `Preparing template ${name}` });
          await this.sleep();
          this.builtTemplates.add(`${wid}/${name}`);
        }
      }
      const conv = this.createConversationSync(wid, base, title);
      this.log("info", `conversation created conversation=${conv.id} branch=${conv.branch} base=${base}`);
      return clone(conv);
    });
  }

  renameConversation(cid: string, title: string) {
    return this.guard("renameConversation", [cid, title], () => {
      this.conv(cid).title = title.trim();
    });
  }

  deleteConversation(cid: string, mode: Parameters<Backend["deleteConversation"]>[1]) {
    return this.guard("deleteConversation", [cid, mode], (): DeleteOutcome => {
      if (this.running.has(cid)) throw "wait for the running turn to finish or cancel it";
      switch (mode.mode) {
        case "check": {
          const unmerged = this.unmerged(cid);
          if (unmerged.length) return { result: "needs_confirmation", unmerged };
          break;
        }
        case "discard":
          break;
        case "merge_into": {
          const out = this.mergeSync(cid, mode.branch);
          if (out.kind === "conflicts") return { result: "merge_conflicts", paths: out.paths };
          break;
        }
        case "keep_copy": {
          if (mode.branch.startsWith("agent/")) throw "keep the copy outside the agent/ namespace";
          const c = this.conv(cid);
          const repo = this.repo(c.workspace_id);
          if (repo.branches.has(mode.branch)) throw `cannot create branch ${mode.branch}: it already exists`;
          repo.branches.set(mode.branch, this.tip(repo, c.branch));
          break;
        }
      }
      this.deleteSync(cid);
      this.log("info", `conversation deleted conversation=${cid}`);
      return { result: "deleted" };
    });
  }

  sendMessage(cid: string, prompt: string) {
    return this.guard("sendMessage", [cid, prompt], async (): Promise<TurnSummary> => {
      const c = this.conv(cid);
      if (this.running.has(cid)) throw "this conversation is already running a turn";
      if (!Object.keys(this.secrets).some((k) => k === "ANTHROPIC_API_KEY" || k === "CLAUDE_CODE_OAUTH_TOKEN")) {
        throw "set ANTHROPIC_API_KEY or CLAUDE_CODE_OAUTH_TOKEN in settings first";
      }
      const run = { cancelled: false };
      this.running.set(cid, run);
      this.log("info", `turn started conversation=${cid}`);
      this.setStatus(cid, "running");
      const entries: TranscriptEntry[] = [{ role: "user", text: prompt }];
      const send = async (event: AgentEvent) => {
        await this.sleep();
        if (run.cancelled) throw new Error("cancelled");
        this.emit({ type: "agent", conversation_id: cid, event });
      };
      const session = c.session_id ?? `sess-${cid}`;
      const reply = `Done. I updated notes.md with: ${prompt}`;
      let isError = false;
      try {
        await send({ type: "session_started", session_id: session, model: "mock", tools: ["Bash", "Edit"] });
        if (/fail/i.test(prompt)) {
          await send({ type: "stderr", text: "simulated failure" });
          await send({ type: "process_exited", code: 1 });
          throw new Error("failed");
        }
        for (const word of "Let me look at the files.".split(/(?= )/)) await send({ type: "text_delta", text: word });
        await send({ type: "assistant_text", text: "Let me look at the files." });
        const toolId = "tu-" + id(6);
        await send({ type: "tool_use", id: toolId, name: "Bash", input: { command: "ls" } });
        entries.push({ role: "tool_use", id: toolId, name: "Bash", input: { command: "ls" } });
        await send({ type: "tool_result", tool_use_id: toolId, content: "README.md\nnotes.md", is_error: false });
        entries.push({ role: "tool_result", tool_use_id: toolId, content: "README.md\nnotes.md", is_error: false });
        if (/slow/i.test(prompt)) for (let i = 0; i < 50; i++) await send({ type: "thinking_delta", text: "." });
        for (const word of reply.split(/(?= )/)) await send({ type: "text_delta", text: word });
        await send({ type: "assistant_text", text: reply });
        entries.push({ role: "assistant", text: reply });
        await send({ type: "turn_completed", is_error: false, result: reply, session_id: session, cost_usd: 0.0123, duration_ms: 1200, num_turns: 2 });
        await send({ type: "process_exited", code: 0 });
      } catch {
        isError = true;
        entries.push({ role: "error", message: run.cancelled ? "cancelled" : "turn failed" });
        if (run.cancelled) this.emit({ type: "agent", conversation_id: cid, event: { type: "process_exited", code: 130 } });
      }
      this.running.delete(cid);
      this.log(isError ? "warn" : "info", `turn finished conversation=${cid} is_error=${isError}`);
      if (!this.conversations.some((x) => x.id === cid)) throw "conversation was deleted";
      const repo = this.repo(c.workspace_id);
      if (!isError) {
        const parent = this.tip(repo, c.branch);
        const files = { ...repo.commits.get(parent)!.files };
        files["notes.md"] = (files["notes.md"] ?? "") + `- ${prompt}\n`;
        const commit = this.commit(repo, c.branch, files, `Agent turn: ${prompt.split("\n")[0].slice(0, 60)}`, [parent]);
        this.emit({ type: "committed", conversation_id: cid, commit });
        if (/learn/i.test(prompt)) {
          const p = this.addPending("skills", "Add skill notes-style", "Learned how notes are kept.", cid, {
            "notes-style/SKILL.md": "---\nname: notes-style\ndescription: Keep notes as a bullet list\n---\n",
          });
          this.emit({ type: "proposal_created", proposal: stripProposal(p) });
          this.emit({ type: "proposal_failed", conversation_id: cid, kind: "tools", error: "test for tool broken failed (exit Some(1))" });
        }
      }
      c.session_id = session;
      if (!isError) {
        const st = this.skillStats.get("rust-style") ?? { uses: 0, successes: 0, failures: 0, negative: 0, last_used: null };
        this.skillStats.set("rust-style", { ...st, uses: st.uses + 1, successes: st.successes + 1, last_used: now() });
        c.last_turn_skills = ["rust-style"];
      } else {
        c.last_turn_skills = [];
      }
      this.transcripts.get(cid)?.push(...entries);
      this.setStatus(cid, isError ? "error" : "idle");
      return { session_id: session, is_error: isError, cost_usd: isError ? null : 0.0123, exit_code: isError ? 1 : 0, entries };
    });
  }

  cancel(cid: string) {
    return this.guard("cancel", [cid], () => {
      const r = this.running.get(cid);
      if (r) r.cancelled = true;
    });
  }

  restartSandbox(cid: string) {
    return this.guard("restartSandbox", [cid], () => {
      if (this.running.has(cid)) throw "wait for the running turn to finish";
      this.conv(cid);
      this.log("info", `sandbox restarted conversation=${cid}`);
    });
  }

  transcript(cid: string) {
    return this.guard("transcript", [cid], () => clone(this.transcripts.get(cid) ?? []));
  }

  conversationDiff(cid: string) {
    return this.guard("conversationDiff", [cid], () => {
      const c = this.conv(cid);
      const repo = this.repo(c.workspace_id);
      const base = this.mergeBase(repo, this.tip(repo, c.base_branch), this.tip(repo, c.branch));
      if (!base) throw `${c.base_branch} and ${c.branch} share no history`;
      return this.diffCommits(repo, base, c.branch);
    });
  }

  unmergedCommits(cid: string) {
    return this.guard("unmergedCommits", [cid], () => this.unmerged(cid));
  }

  mergeConversation(cid: string, into: string) {
    return this.guard("mergeConversation", [cid, into], () => {
      if (this.running.has(cid)) throw "wait for the running turn to finish";
      return this.mergeSync(cid, into);
    });
  }

  proposals() {
    return this.guard("proposals", [], () =>
      (["skills", "tools", "templates"] as LibraryKind[]).flatMap((k) => this.libraries[k].pending.map(stripProposal)).sort((a, b) => b.created - a.created),
    );
  }

  proposal(kind: LibraryKind, pid: string) {
    return this.guard("proposal", [kind, pid], () => {
      const p = this.libraries[kind].pending.find((p) => p.id === pid);
      if (!p) throw `no pending proposal ${pid}`;
      const diff: FileDiff[] = Object.entries(p.files).map(([path, content]) => {
        const old = this.libraries[kind].files[path];
        const { patch, additions, deletions } = patchFor(old, content ?? undefined);
        return { path, old_path: null, status: content === null ? "deleted" : old === undefined ? "added" : "modified", binary: false, additions, deletions, patch };
      });
      return { proposal: stripProposal(p), diff };
    });
  }

  approve(kind: LibraryKind, pid: string) {
    return this.guard("approve", [kind, pid], () => {
      const lib = this.libraries[kind];
      const p = lib.pending.find((p) => p.id === pid);
      if (!p) throw `no pending proposal ${pid}`;
      for (const [path, content] of Object.entries(p.files)) {
        if (content === null) delete lib.files[path];
        else lib.files[path] = content;
      }
      lib.pending = lib.pending.filter((x) => x.id !== pid);
      const c = this.libCommit(`Approve: ${p.title}`);
      (c as MockCommit & { change?: Record<string, string | null> }).change = p.files;
      lib.history.unshift(c);
      return c.id;
    });
  }

  reject(kind: LibraryKind, pid: string) {
    return this.guard("reject", [kind, pid], () => {
      const lib = this.libraries[kind];
      if (!lib.pending.some((p) => p.id === pid)) throw `no pending proposal ${pid}`;
      lib.pending = lib.pending.filter((p) => p.id !== pid);
    });
  }

  history(kind: LibraryKind, limit: number) {
    return this.guard("history", [kind, limit], () => this.libraries[kind].history.slice(0, limit).map(strip));
  }

  revert(kind: LibraryKind, commit: string) {
    return this.guard("revert", [kind, commit], () => {
      const lib = this.libraries[kind];
      const c = lib.history.find((h) => h.id === commit) as (MockCommit & { change?: Record<string, string | null> }) | undefined;
      if (!c) throw `unknown commit ${commit}`;
      for (const path of Object.keys(c.change ?? {})) delete lib.files[path];
      const r = this.libCommit(`Revert "${c.summary}"`);
      lib.history.unshift(r);
      return r.id;
    });
  }

  skills() {
    return this.guard("skills", [], (): SkillSummary[] =>
      Object.entries(this.libraries.skills.files)
        .filter(([p]) => p.endsWith("/SKILL.md"))
        .map(([path, text]) => {
          const name = path.split("/")[0];
          const description = /description:\s*(.*)/.exec(text)?.[1] ?? "";
          const when = /when_to_use:\s*(.*)/.exec(text)?.[1] ?? null;
          return { name, description, when_to_use: when, stats: { ...(this.skillStats.get(name) ?? { uses: 0, successes: 0, failures: 0, negative: 0, last_used: null }) } };
        })
        .sort((a, b) => a.name.localeCompare(b.name)),
    );
  }

  tools() {
    return this.guard("tools", [], (): ToolManifest[] =>
      Object.entries(this.libraries.tools.files)
        .filter(([p]) => p.endsWith("/tool.toml"))
        .map(([, text]) => {
          const get = (k: string) => new RegExp(`^${k}\\s*=\\s*"([^"]*)"`, "m").exec(text)?.[1];
          return { name: get("name") ?? "", description: get("description") ?? "", run: get("run") ?? "", test: get("test") ?? null, timeout_secs: Number(/^timeout_secs\s*=\s*(\d+)/m.exec(text)?.[1] ?? 120) };
        })
        .sort((a, b) => a.name.localeCompare(b.name)),
    );
  }

  skillSource(name: string) {
    return this.guard("skillSource", [name], () => {
      const text = this.libraries.skills.files[`${name}/SKILL.md`];
      if (text === undefined) throw `no skill named ${name}`;
      return text;
    });
  }

  templateSource(name: string) {
    return this.guard("templateSource", [name], () => {
      const text = this.libraries.templates.files[`${name}/template.toml`];
      if (text === undefined) throw `no template named ${name}`;
      return text;
    });
  }

  proposeSkill(content: string, rationale: string) {
    return this.guard("proposeSkill", [content, rationale], () => {
      const m = /^---\n([\s\S]*?)\n---/.exec(content);
      if (!m) throw "SKILL.md must start with '---' frontmatter";
      const name = /^name:\s*(.+)$/m.exec(m[1])?.[1].trim();
      const description = /^description:\s*(.+)$/m.exec(m[1])?.[1].trim();
      if (!name || !description) throw "frontmatter needs `name` and `description`";
      if (!/^[a-z0-9][a-z0-9-]{0,63}$/.test(name)) throw `skill name ${JSON.stringify(name)} must be lowercase letters, digits and '-', at most 64 characters`;
      const exists = this.libraries.skills.files[`${name}/SKILL.md`] !== undefined;
      return stripProposal(this.addPending("skills", `${exists ? "Update" : "Add"} skill ${name}`, rationale, null, { [`${name}/SKILL.md`]: content }));
    });
  }

  proposeSkillRemoval(name: string, rationale: string) {
    return this.guard("proposeSkillRemoval", [name, rationale], () => {
      if (this.libraries.skills.files[`${name}/SKILL.md`] === undefined) throw `no skill named ${name}`;
      return stripProposal(this.addPending("skills", `Remove skill ${name}`, rationale, null, { [`${name}/SKILL.md`]: null }));
    });
  }

  proposeTemplate(manifest: string, rationale: string) {
    return this.guard("proposeTemplate", [manifest, rationale], () => {
      const t = parseTemplateToml(manifest);
      const exists = this.libraries.templates.files[`${t.name}/template.toml`] !== undefined;
      return stripProposal(this.addPending("templates", `${exists ? "Update" : "Add"} template ${t.name}`, rationale, null, { [`${t.name}/template.toml`]: manifest }));
    });
  }

  markLastTurnWrong(cid: string) {
    return this.guard("markLastTurnWrong", [cid], () => {
      const c = this.conv(cid);
      const skills = c.last_turn_skills;
      c.last_turn_skills = [];
      for (const name of skills) {
        const st = this.skillStats.get(name) ?? { uses: 0, successes: 0, failures: 0, negative: 0, last_used: null };
        this.skillStats.set(name, { ...st, successes: Math.max(0, st.successes - 1), negative: st.negative + 1 });
      }
      return skills;
    });
  }

  cleanupOrphans() {
    return this.guard("cleanupOrphans", [], (): CleanupReport => ({ containers: [], worktrees: [], branches: [], errors: [] }));
  }

  buildAgentImage() {
    return this.guard("buildAgentImage", [], async () => {
      await this.sleep();
      return "Successfully tagged localhost/nucleus-agent:latest";
    });
  }

  recentLogs(minLevel: LogLevel | null) {
    const rank = { trace: 0, debug: 1, info: 2, warn: 3, error: 4 } as const;
    return this.guard("recentLogs", [minLevel], () => this.logs.filter((l) => rank[l.level] >= rank[minLevel ?? "trace"]).map((l) => ({ ...l })));
  }

  subscribe(listener: (e: HarnessEvent) => void) {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  }
}

function strip(c: MockCommit): CommitInfo {
  const { id, summary, message, author_name, author_email, time, parents } = c;
  return { id, summary, message, author_name, author_email, time, parents: [...parents] };
}

function stripProposal(p: PendingProposal): Proposal {
  const { id, kind, title, rationale, source, commit, created } = p;
  return { id, kind, title, rationale, source, commit, created };
}
