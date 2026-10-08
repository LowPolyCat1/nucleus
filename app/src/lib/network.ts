import type { NetworkPolicy } from "../api/types";

const HOST_RE = /^(\*\.)?([a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?)(\.[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?)*$/;

export interface ParsedHosts {
  hosts: string[];
  invalid: string[];
}

/** Parse a free-form allowlist (one host per line or comma separated, URLs tolerated). */
export function parseHosts(text: string): ParsedHosts {
  const hosts: string[] = [];
  const invalid: string[] = [];
  for (const raw of text.split(/[\s,]+/)) {
    if (!raw) continue;
    let h = raw.trim().toLowerCase();
    h = h.replace(/^[a-z]+:\/\//, "").split("/")[0].replace(/:\d+$/, "");
    if (h === "*" || HOST_RE.test(h)) {
      if (!hosts.includes(h)) hosts.push(h);
    } else {
      invalid.push(raw);
    }
  }
  return { hosts, invalid };
}

export function describePolicy(p: NetworkPolicy): string {
  switch (p.mode) {
    case "none":
      return "No network (model API only)";
    case "full":
      return "Full network access";
    case "allowlist":
      return p.hosts.length ? `Allowlist: ${p.hosts.join(", ")}` : "Allowlist (empty, model API only)";
  }
}
