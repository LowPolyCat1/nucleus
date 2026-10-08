import { PERMISSION_MODES, type Settings } from "../api/types";

/** Same rules as `Settings::validate` in crates/harness/src/state.rs. Returns an error or null. */
export function validateSettings(s: Settings): string | null {
  if (!s.image.trim()) return "image must not be empty";
  if (!(PERMISSION_MODES as readonly string[]).includes(s.permission_mode)) return `permission mode must be one of ${PERMISSION_MODES.join(", ")}`;
  const { memory_mb, cpus, pids } = s.limits;
  if (memory_mb !== null && !(Number.isInteger(memory_mb) && memory_mb >= 256 && memory_mb <= 1024 * 1024)) return "memory limit must be between 256 MiB and 1 TiB";
  if (cpus !== null && !(Number.isFinite(cpus) && cpus >= 0.1 && cpus <= 1024)) return "CPU limit must be between 0.1 and 1024 cores";
  if (pids !== null && !(Number.isInteger(pids) && pids >= 64 && pids <= 1_000_000)) return "process limit must be between 64 and 1000000";
  return null;
}

/** Parse a number input; blank means no limit. `NaN` for garbage, so validation catches it. */
export function parseLimit(text: string): number | null {
  const t = text.trim();
  if (!t) return null;
  const n = Number(t);
  return Number.isFinite(n) ? n : NaN;
}

export const PERMISSION_HELP: Record<string, string> = {
  bypassPermissions: "Use every tool without asking. The sandbox is the boundary. Recommended.",
  acceptEdits: "Edit files freely; other tools that need approval are denied (nobody can approve in a headless run).",
  auto: "Let the CLI decide per action; actions it would ask about are denied.",
  dontAsk: "Deny anything that is not pre-approved.",
  manual: "Ask for every action; in a headless run that denies all tools.",
  plan: "Plan only, no changes.",
};
