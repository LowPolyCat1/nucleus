import { describe, expect, test } from "vitest";
import type { AgentEvent } from "../../src/api/types";
import { applyEvent, emptyChat, fromTranscript, startTurn, toolSummary } from "../../src/lib/chat";

const run = (events: AgentEvent[], state = startTurn(emptyChat(), "hi")) => events.reduce(applyEvent, state);

describe("applyEvent", () => {
  test("streams deltas then replaces them with the complete block", () => {
    const s = run([
      { type: "session_started", session_id: "s1", model: null, tools: [] },
      { type: "text_delta", text: "Hel" },
      { type: "text_delta", text: "lo" },
    ]);
    expect(s.sessionId).toBe("s1");
    expect(s.items[1]).toEqual({ kind: "assistant", text: "Hello", streaming: true });
    const done = applyEvent(s, { type: "assistant_text", text: "Hello!" });
    expect(done.items).toHaveLength(2);
    expect(done.items[1]).toEqual({ kind: "assistant", text: "Hello!", streaming: false });
  });

  test("assistant_text without deltas appends", () => {
    const s = run([{ type: "assistant_text", text: "A" }, { type: "assistant_text", text: "B" }]);
    expect(s.items.map((i) => i.kind)).toEqual(["user", "assistant", "assistant"]);
  });

  test("deltas after a finished block start a new block", () => {
    const s = run([{ type: "assistant_text", text: "A" }, { type: "text_delta", text: "B" }]);
    expect(s.items).toHaveLength(3);
    expect(s.items[2]).toMatchObject({ text: "B", streaming: true });
  });

  test("empty deltas are ignored and do not create items", () => {
    const s0 = startTurn(emptyChat(), "x");
    expect(applyEvent(s0, { type: "text_delta", text: "" })).toBe(s0);
  });

  test("thinking and text stream into separate items", () => {
    const s = run([
      { type: "thinking_delta", text: "hmm" },
      { type: "text_delta", text: "ok" },
      { type: "thinking_delta", text: " more" },
    ]);
    expect(s.items.slice(1)).toEqual([
      { kind: "thinking", text: "hmm more", streaming: true },
      { kind: "assistant", text: "ok", streaming: true },
    ]);
  });

  test("tool results attach to their call, unknown ids become orphan items", () => {
    const s = run([
      { type: "text_delta", text: "let me look" },
      { type: "tool_use", id: "t1", name: "Bash", input: { command: "ls" } },
      { type: "tool_use", id: "t2", name: "Read", input: { file_path: "a" } },
      { type: "tool_result", tool_use_id: "t1", content: "a\nb", is_error: false },
      { type: "tool_result", tool_use_id: "zz", content: "?", is_error: true },
    ]);
    expect(s.items[1]).toMatchObject({ kind: "assistant", streaming: false });
    expect(s.items[2]).toMatchObject({ kind: "tool", id: "t1", result: "a\nb", isError: false });
    expect(s.items[3]).toMatchObject({ kind: "tool", id: "t2", result: null });
    expect(s.items[4]).toMatchObject({ kind: "tool", id: "zz", name: "unknown", isError: true });
  });

  test("turn completion closes streams and records cost", () => {
    const s = run([
      { type: "text_delta", text: "x" },
      { type: "turn_completed", is_error: false, result: "x", session_id: "s2", cost_usd: 0.5, duration_ms: 10, num_turns: 1 },
      { type: "process_exited", code: 0 },
    ]);
    expect(s.completed).toBe(true);
    expect(s.sessionId).toBe("s2");
    expect(s.items[1]).toMatchObject({ streaming: false });
    expect(s.items.at(-1)).toEqual({ kind: "turn", isError: false, costUsd: 0.5, durationMs: 10 });
  });

  test("a crash before completion becomes an error, exit after completion does not", () => {
    const crashed = run([{ type: "text_delta", text: "x" }, { type: "process_exited", code: 2 }]);
    expect(crashed.items.at(-1)).toMatchObject({ kind: "error" });
    expect((crashed.items.at(-1) as { message: string }).message).toContain("code 2");
    const nullCode = run([{ type: "process_exited", code: null }]);
    expect((nullCode.items.at(-1) as { message: string }).message).toContain("unknown");
    const ok = run([
      { type: "turn_completed", is_error: true, result: null, session_id: null, cost_usd: null, duration_ms: null, num_turns: null },
      { type: "process_exited", code: 1 },
    ]);
    expect(ok.items.filter((i) => i.kind === "error")).toHaveLength(0);
  });

  test("errors and stderr", () => {
    const s0 = startTurn(emptyChat(), "x");
    expect(applyEvent(s0, { type: "stderr", text: "noise" })).toBe(s0);
    expect(applyEvent(s0, { type: "error", message: "bad" }).items.at(-1)).toEqual({ kind: "error", message: "bad" });
  });

  test("does not mutate the input state", () => {
    const s0 = run([{ type: "text_delta", text: "a" }]);
    const copy = JSON.parse(JSON.stringify(s0));
    applyEvent(s0, { type: "text_delta", text: "b" });
    applyEvent(s0, { type: "assistant_text", text: "z" });
    expect(s0).toEqual(copy);
  });
});

describe("fromTranscript", () => {
  test("rebuilds items", () => {
    const s = fromTranscript([
      { role: "user", text: "q" },
      { role: "assistant", text: "a" },
      { role: "tool_use", id: "t", name: "Bash", input: {} },
      { role: "tool_result", tool_use_id: "t", content: "out", is_error: false },
      { role: "error", message: "e" },
      { role: "user", text: "q2" },
    ]);
    expect(s.items.map((i) => i.kind)).toEqual(["user", "assistant", "tool", "error", "user"]);
    expect(s.items[2]).toMatchObject({ result: "out" });
    expect(s.completed).toBe(true);
  });
  test("empty transcript", () => {
    expect(fromTranscript([]).items).toEqual([]);
  });
});

describe("toolSummary", () => {
  test("picks the most descriptive field, first line, truncated", () => {
    expect(toolSummary("Bash", { command: "ls -la\nrm x" })).toBe("ls -la");
    expect(toolSummary("Read", { file_path: "/a/b" })).toBe("/a/b");
    expect(toolSummary("Bash", { command: "x".repeat(300) })).toHaveLength(120);
    expect(toolSummary("Weird", null)).toBe("Weird");
    expect(toolSummary("Weird", { command: 5 })).toBe("Weird");
    expect(toolSummary("Weird", { command: "" })).toBe("Weird");
  });
});
