import { describe, expect, test } from "vitest";
import { SECRET_MASK } from "../../src/api/backend";
import { MockBackend } from "../../src/api/mock";
import type { HarnessEvent } from "../../src/api/types";

const ready = async (m: MockBackend) => {
  await m.updateSettings({ ...(await m.state()).settings, provider_env: { ANTHROPIC_API_KEY: "sk" } });
};

describe("MockBackend", () => {
  test("settings mask secrets and keep them when the mask is sent back", async () => {
    const m = new MockBackend({ delayMs: 0 });
    await ready(m);
    const s = (await m.state()).settings;
    expect(s.provider_env).toEqual({ ANTHROPIC_API_KEY: SECRET_MASK });
    await m.updateSettings({ ...s, model: "x" });
    expect((await m.state()).settings.provider_env).toEqual({ ANTHROPIC_API_KEY: SECRET_MASK });
    await m.updateSettings({ ...s, provider_env: { ANTHROPIC_API_KEY: "" } });
    expect((await m.state()).settings.provider_env).toEqual({});
    await expect(m.updateSettings({ ...s, image: " " })).rejects.toContain("image");
  });

  test("a turn streams events, commits and records the session", async () => {
    const m = new MockBackend({ delayMs: 0 });
    await ready(m);
    const [conv] = (await m.state()).conversations;
    const events: HarnessEvent[] = [];
    m.subscribe((e) => events.push(e));
    const before = (await m.unmergedCommits(conv.id)).length;
    const summary = await m.sendMessage(conv.id, "write notes");
    expect(summary.is_error).toBe(false);
    expect(events.map((e) => e.type)).toContain("committed");
    expect(events.filter((e) => e.type === "status").map((e) => (e as { status: string }).status)).toEqual(["running", "idle"]);
    expect((await m.unmergedCommits(conv.id)).length).toBe(before + 1);
    expect((await m.conversationDiff(conv.id)).map((f) => f.path)).toContain("notes.md");
    expect((await m.state()).conversations[0].session_id).toBe(summary.session_id);
  });

  test("turn guards: credentials, concurrency, cancel", async () => {
    const m = new MockBackend({ delayMs: 5 });
    const [conv] = (await m.state()).conversations;
    await expect(m.sendMessage(conv.id, "x")).rejects.toContain("ANTHROPIC_API_KEY");
    await ready(m);
    const first = m.sendMessage(conv.id, "slow please");
    await new Promise((r) => setTimeout(r, 20));
    await expect(m.sendMessage(conv.id, "again")).rejects.toContain("already running");
    await expect(m.deleteConversation(conv.id, { mode: "discard" })).rejects.toContain("running");
    await m.cancel(conv.id);
    expect((await first).is_error).toBe(true);
    expect((await m.state()).conversations[0].status).toBe("error");
    await m.cancel(conv.id); // idle cancel is harmless
  });

  test("delete flow with unmerged work, keep copy and merge conflicts", async () => {
    const m = new MockBackend({ delayMs: 0 });
    const s = await m.state();
    const ws = s.workspaces[0];
    const [conv] = s.conversations;
    const check = await m.deleteConversation(conv.id, { mode: "check" });
    expect(check.result).toBe("needs_confirmation");
    await expect(m.deleteConversation(conv.id, { mode: "keep_copy", branch: "agent/x" })).rejects.toContain("agent/");
    await expect(m.deleteConversation(conv.id, { mode: "keep_copy", branch: "main" })).rejects.toContain("exists");
    // Conflict: main changes the same file differently.
    m.commitOnBranch(ws.id, "main", "src/lib.rs", "different\n");
    expect(await m.deleteConversation(conv.id, { mode: "merge_into", branch: "main" })).toEqual({ result: "merge_conflicts", paths: ["src/lib.rs"] });
    expect(await m.deleteConversation(conv.id, { mode: "keep_copy", branch: "local/kept" })).toEqual({ result: "deleted" });
    expect((await m.branches(ws.id)).map((b) => b.name)).toContain("local/kept");
    expect((await m.state()).conversations).toEqual([]);
  });

  test("merge fast-forwards, merges and reports up to date", async () => {
    const m = new MockBackend({ delayMs: 0 });
    const s = await m.state();
    const ws = s.workspaces[0];
    const conv = s.conversations[0];
    expect((await m.mergeConversation(conv.id, "main")).kind).toBe("fast_forward");
    expect((await m.mergeConversation(conv.id, "main")).kind).toBe("up_to_date");
    await expect(m.mergeConversation(conv.id, "agent/zzz")).rejects.toContain("local branch");
    const c2 = await m.createConversation(ws.id, "main", "two");
    m.commitOnBranch(ws.id, c2.branch, "b.txt", "b\n");
    m.commitOnBranch(ws.id, "main", "c.txt", "c\n");
    const out = await m.mergeConversation(c2.id, "main");
    expect(out.kind).toBe("merged");
    expect((await m.unmergedCommits(c2.id)).length).toBe(0);
    expect((await m.deleteConversation(c2.id, { mode: "check" })).result).toBe("deleted");
  });

  test("workspaces and templates validation", async () => {
    const m = new MockBackend({ delayMs: 0, seed: false });
    await expect(m.addWorkspace("relative/path", null)).rejects.toContain("no git repository");
    const ws = await m.addWorkspace("/a/b/", null);
    expect(ws.name).toBe("b");
    await expect(m.addWorkspace("/a/b", null)).rejects.toContain("already");
    await expect(m.configureWorkspace(ws.id, ["python", "python-alt"], { mode: "none" })).rejects.toContain("both set VIRTUAL_ENV");
    await expect(m.configureWorkspace(ws.id, ["nope"], { mode: "none" })).rejects.toContain("not found");
    await m.configureWorkspace(ws.id, ["node", "python"], { mode: "allowlist", hosts: ["pypi.org"] });
    expect((await m.templateStatus(ws.id)).every((t) => !t.fresh)).toBe(true);
    await m.buildTemplates(ws.id);
    expect((await m.templateStatus(ws.id)).every((t) => t.fresh)).toBe(true);
    const c = await m.createConversation(ws.id, "main", "  ");
    expect(c.title).toBe("New conversation");
    await expect(m.createConversation(ws.id, "nope", "t")).rejects.toContain("cannot resolve");
    await expect(m.removeWorkspace(ws.id)).rejects.toContain("conversations");
    await m.deleteConversation(c.id, { mode: "check" });
    await m.removeWorkspace(ws.id);
    expect((await m.state()).workspaces).toEqual([]);
  });

  test("proposals approve, reject, revert", async () => {
    const m = new MockBackend({ delayMs: 0 });
    const ps = await m.proposals();
    expect(ps.map((p) => p.kind).sort()).toEqual(["skills", "tools"]);
    const skill = ps.find((p) => p.kind === "skills")!;
    const detail = await m.proposal("skills", skill.id);
    expect(detail.diff[0].status).toBe("added");
    const commit = await m.approve("skills", skill.id);
    expect((await m.skills()).map((s) => s.name)).toContain("cargo-tests");
    await expect(m.approve("skills", skill.id)).rejects.toContain("no pending");
    await m.revert("skills", commit);
    expect((await m.skills()).map((s) => s.name)).not.toContain("cargo-tests");
    const tool = ps.find((p) => p.kind === "tools")!;
    await m.reject("tools", tool.id);
    expect(await m.proposals()).toEqual([]);
  });

  test("failure injection and init errors", async () => {
    const m = new MockBackend({ delayMs: 0, initError: "no engine" });
    expect((await m.init()).error).toBe("no engine");
    m.setInitError(null);
    expect((await m.init()).ready).toBe(true);
    m.failNext("state", "boom");
    await expect(m.state()).rejects.toBe("boom");
    await expect(m.state()).resolves.toBeTruthy();
    expect(m.calls.filter(([c]) => c === "state")).toHaveLength(2);
  });

  test("graph covers all branches and diffs are well formed", async () => {
    const m = new MockBackend({ delayMs: 0 });
    const ws = (await m.state()).workspaces[0];
    const graph = await m.graph(ws.id, 100);
    expect(graph.length).toBeGreaterThanOrEqual(4);
    expect(await m.graph(ws.id, 1)).toHaveLength(1);
    const d = await m.diff(ws.id, "main", "local/feature");
    expect(d).toHaveLength(1);
    expect(d[0].patch).toMatch(/^@@ -0,0 \+1,1 @@\n\+# Feature\n$/);
    expect(await m.diff(ws.id, "main", "main")).toEqual([]);
  });
});

describe("library authoring in the mock", () => {
  test("template TOML round-trips and validates", async () => {
    const { parseTemplateToml, templateToToml } = await import("../../src/api/mock");
    const m = new MockBackend({ delayMs: 0 });
    for (const t of await m.availableTemplates()) {
      const back = parseTemplateToml(templateToToml(t));
      expect(back.name).toBe(t.name);
      expect(back.mount).toEqual(t.mount);
      expect(back.env).toEqual(t.env);
    }
    expect(() => parseTemplateToml('name = "a b"')).toThrow(/letters, digits/);
    expect(() => parseTemplateToml('name = "a"\n[build]\ncommand = "x"')).toThrow(/mount/);
    expect(() => parseTemplateToml('name = "a"\nmount = { mode = "worktree", path = "../x" }\n[build]\ncommand = "x"')).toThrow(/relative/);
    expect(() => parseTemplateToml('name = "a"\nmount = { mode = "readonly" }')).toThrow(/build/);
  });

  test("propose skill, removal, template; mark wrong", async () => {
    const m = new MockBackend({ delayMs: 0 });
    await expect(m.proposeSkill("nope", "r")).rejects.toContain("frontmatter");
    await expect(m.proposeSkill("---\nname: Bad\ndescription: d\n---\n", "r")).rejects.toContain("lowercase");
    const p = await m.proposeSkill("---\nname: new-one\ndescription: d\n---\nbody\n", "r");
    expect(p.title).toBe("Add skill new-one");
    expect((await m.proposeSkill("---\nname: rust-style\ndescription: d\n---\n", "r")).title).toBe("Update skill rust-style");
    await expect(m.proposeSkillRemoval("missing", "r")).rejects.toContain("no skill");
    expect((await m.proposeSkillRemoval("rust-style", "r")).title).toBe("Remove skill rust-style");
    const t = await m.proposeTemplate('name = "go"\nmount = { mode = "readonly" }\n[build]\ncommand = "true"\n', "r");
    expect(t.title).toBe("Add template go");
    await m.approve("templates", t.id);
    expect((await m.availableTemplates()).map((x) => x.name)).toContain("go");
    expect(await m.templateSource("go")).toContain('name = "go"');
    await expect(m.templateSource("nope")).rejects.toContain("no template");
    await expect(m.skillSource("nope")).rejects.toContain("no skill");
    expect((await m.tools()).map((x) => x.name)).toEqual(["word-count"]);

    await withKeyMock(m);
    const [conv] = (await m.state()).conversations;
    await m.sendMessage(conv.id, "x");
    expect((await m.state()).conversations[0].last_turn_skills).toEqual(["rust-style"]);
    const before = (await m.skills()).find((s) => s.name === "rust-style")!.stats;
    expect(await m.markLastTurnWrong(conv.id)).toEqual(["rust-style"]);
    const after = (await m.skills()).find((s) => s.name === "rust-style")!.stats;
    expect([after.successes, after.negative]).toEqual([before.successes - 1, before.negative + 1]);
    expect(await m.markLastTurnWrong(conv.id)).toEqual([]);
  });
});

async function withKeyMock(m: MockBackend) {
  await m.updateSettings({ ...(await m.state()).settings, provider_env: { ANTHROPIC_API_KEY: "k" } });
}
