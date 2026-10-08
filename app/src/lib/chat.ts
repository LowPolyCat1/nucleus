import type { AgentEvent, TranscriptEntry } from "../api/types";

export type ChatItem =
  | { kind: "user"; text: string }
  | { kind: "assistant"; text: string; streaming: boolean }
  | { kind: "thinking"; text: string; streaming: boolean }
  | { kind: "tool"; id: string; name: string; input: unknown; result: string | null; isError: boolean }
  | { kind: "error"; message: string }
  | { kind: "turn"; isError: boolean; costUsd: number | null; durationMs: number | null };

export interface ChatState {
  items: ChatItem[];
  sessionId: string | null;
  /** True once a `turn_completed` event arrived for the current turn. */
  completed: boolean;
}

export const emptyChat = (): ChatState => ({ items: [], sessionId: null, completed: false });

function lastStreaming(items: ChatItem[], kind: "assistant" | "thinking"): number {
  for (let i = items.length - 1; i >= 0; i--) {
    const it = items[i];
    if (it.kind === kind) return it.streaming ? i : -1;
    // A streaming block continues across the other text kind only (text and thinking
    // interleave); anything else ends it.
    if (it.kind !== "thinking" && it.kind !== "assistant") return -1;
  }
  return -1;
}

function finishStreaming(items: ChatItem[]): ChatItem[] {
  return items.map((it) => (it.kind === "assistant" || it.kind === "thinking") && it.streaming ? { ...it, streaming: false } : it);
}

/** Pure reducer from agent events to chat items. Never mutates its input. */
export function applyEvent(state: ChatState, ev: AgentEvent): ChatState {
  const items = state.items.slice();
  switch (ev.type) {
    case "session_started":
      return { ...state, sessionId: ev.session_id || state.sessionId, completed: false };
    case "text_delta":
    case "thinking_delta": {
      if (!ev.text) return state;
      const kind = ev.type === "text_delta" ? "assistant" : "thinking";
      const i = lastStreaming(items, kind);
      if (i >= 0) {
        const it = items[i] as Extract<ChatItem, { kind: "assistant" | "thinking" }>;
        items[i] = { ...it, text: it.text + ev.text };
      } else {
        items.push({ kind, text: ev.text, streaming: true });
      }
      return { ...state, items };
    }
    case "assistant_text": {
      // The complete block supersedes the streamed deltas for it.
      const i = lastStreaming(items, "assistant");
      if (i >= 0) items[i] = { kind: "assistant", text: ev.text, streaming: false };
      else items.push({ kind: "assistant", text: ev.text, streaming: false });
      return { ...state, items: finishThinking(items) };
    }
    case "tool_use":
      return { ...state, items: [...finishStreaming(items), { kind: "tool", id: ev.id, name: ev.name, input: ev.input, result: null, isError: false }] };
    case "tool_result": {
      const i = items.findIndex((it) => it.kind === "tool" && it.id === ev.tool_use_id);
      if (i >= 0) {
        items[i] = { ...(items[i] as Extract<ChatItem, { kind: "tool" }>), result: ev.content, isError: ev.is_error };
      } else {
        items.push({ kind: "tool", id: ev.tool_use_id, name: "unknown", input: null, result: ev.content, isError: ev.is_error });
      }
      return { ...state, items };
    }
    case "turn_completed":
      return {
        ...state,
        sessionId: ev.session_id ?? state.sessionId,
        completed: true,
        items: [...finishStreaming(items), { kind: "turn", isError: ev.is_error, costUsd: ev.cost_usd, durationMs: ev.duration_ms }],
      };
    case "error":
      return { ...state, items: [...finishStreaming(items), { kind: "error", message: ev.message }] };
    case "process_exited":
      if (ev.code !== 0 && !state.completed) {
        return { ...state, items: [...finishStreaming(items), { kind: "error", message: `The agent process exited with code ${ev.code ?? "unknown"} before finishing the turn.` }] };
      }
      return { ...state, items: finishStreaming(items) };
    case "stderr":
      return state;
  }
}

function finishThinking(items: ChatItem[]): ChatItem[] {
  return items.map((it) => (it.kind === "thinking" && it.streaming ? { ...it, streaming: false } : it));
}

/** Start a new turn: append the user's message. */
export function startTurn(state: ChatState, prompt: string): ChatState {
  return { ...state, completed: false, items: [...state.items, { kind: "user", text: prompt }] };
}

/** Rebuild chat items from a persisted transcript. */
export function fromTranscript(entries: TranscriptEntry[]): ChatState {
  let state = emptyChat();
  for (const e of entries) {
    switch (e.role) {
      case "user":
        state = startTurn(state, e.text);
        break;
      case "assistant":
        state = applyEvent(state, { type: "assistant_text", text: e.text });
        break;
      case "tool_use":
        state = applyEvent(state, { type: "tool_use", id: e.id, name: e.name, input: e.input });
        break;
      case "tool_result":
        state = applyEvent(state, { type: "tool_result", tool_use_id: e.tool_use_id, content: e.content, is_error: e.is_error });
        break;
      case "error":
        state = applyEvent(state, { type: "error", message: e.message });
        break;
    }
  }
  return { ...state, completed: true };
}

/** Short human summary of a tool call's input, e.g. the command of a Bash call. */
export function toolSummary(name: string, input: unknown): string {
  if (input && typeof input === "object") {
    const o = input as Record<string, unknown>;
    for (const key of ["command", "file_path", "path", "pattern", "skill", "url", "query", "description"]) {
      if (typeof o[key] === "string" && o[key]) return String(o[key]).split("\n")[0].slice(0, 120);
    }
  }
  return name;
}
