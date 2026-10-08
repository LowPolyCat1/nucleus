export function shortId(id: string, n = 7): string {
  return id.slice(0, n);
}

/** Relative time like "5m ago" for unix seconds. `now` is injectable for tests. */
export function relativeTime(unixSeconds: number, now = Date.now() / 1000): string {
  const d = Math.max(0, Math.floor(now - unixSeconds));
  if (d < 60) return "just now";
  if (d < 3600) return `${Math.floor(d / 60)}m ago`;
  if (d < 86400) return `${Math.floor(d / 3600)}h ago`;
  if (d < 86400 * 30) return `${Math.floor(d / 86400)}d ago`;
  return new Date(unixSeconds * 1000).toISOString().slice(0, 10);
}

export function formatCost(usd: number | null | undefined): string {
  if (usd == null || !Number.isFinite(usd)) return "";
  if (usd < 0.01) return "<$0.01";
  return `$${usd.toFixed(2)}`;
}

/** Error from IPC (string), Error or anything else, as a message. */
export function errorMessage(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  if (e && typeof e === "object" && "message" in e) return String((e as { message: unknown }).message);
  return String(e);
}

/** Laplace-smoothed reliability as the Rust side computes it. */
export function reliability(s: { successes: number; failures: number; negative: number }): number {
  return (s.successes + 1) / (s.successes + s.failures + s.negative + 2);
}
