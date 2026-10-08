export type DiffLineKind = "add" | "del" | "ctx" | "meta";

export interface DiffLine {
  kind: DiffLineKind;
  text: string;
  oldNo: number | null;
  newNo: number | null;
}

export interface Hunk {
  header: string;
  oldStart: number;
  newStart: number;
  lines: DiffLine[];
}

const HUNK_RE = /^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@(.*)$/;

/** Parse unified diff hunks (as produced by the vcs crate, headers optional). */
export function parsePatch(patch: string): Hunk[] {
  const hunks: Hunk[] = [];
  let current: Hunk | null = null;
  let oldNo = 0;
  let newNo = 0;
  const lines = patch.split("\n");
  // A trailing newline produces one empty element that is not a context line.
  if (lines.length && lines[lines.length - 1] === "") lines.pop();
  for (const raw of lines) {
    const m = HUNK_RE.exec(raw);
    if (m) {
      oldNo = Number(m[1]);
      newNo = Number(m[3]);
      current = { header: raw, oldStart: oldNo, newStart: newNo, lines: [] };
      hunks.push(current);
      continue;
    }
    if (!current) continue; // file headers (---/+++) or junk before the first hunk
    const sign = raw[0];
    const text = raw.slice(1);
    if (sign === "+") current.lines.push({ kind: "add", text, oldNo: null, newNo: newNo++ });
    else if (sign === "-") current.lines.push({ kind: "del", text, oldNo: oldNo++, newNo: null });
    else if (sign === "\\") current.lines.push({ kind: "meta", text: raw, oldNo: null, newNo: null });
    else current.lines.push({ kind: "ctx", text: sign === " " ? text : raw, oldNo: oldNo++, newNo: newNo++ });
  }
  return hunks;
}

export function diffStats(files: { additions: number; deletions: number }[]): { additions: number; deletions: number } {
  return files.reduce((a, f) => ({ additions: a.additions + f.additions, deletions: a.deletions + f.deletions }), { additions: 0, deletions: 0 });
}
