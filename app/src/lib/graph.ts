import type { BranchInfo, CommitInfo } from "../api/types";

export interface GraphRow {
  commit: CommitInfo;
  /** Lane the commit's node sits in. */
  lane: number;
  /** Lanes occupied (by expected commit id) when the row starts. */
  lanesIn: (string | null)[];
  /** Lanes occupied after the row. */
  lanesOut: (string | null)[];
  /** Lines from this node to its parents' lanes. */
  parentLanes: number[];
  /** Branches pointing at this commit. */
  refs: BranchInfo[];
}

/**
 * Assign commits (newest first, as `Vcs::graph` returns them) to lanes for a branch tree.
 * Lanes whose expected commit never shows up (history cut off by the limit) stay open to the
 * bottom; duplicate lanes converging on one commit are closed at that commit.
 */
export function layoutGraph(commits: CommitInfo[], branches: BranchInfo[] = []): GraphRow[] {
  const lanes: (string | null)[] = [];
  const rows: GraphRow[] = [];
  const refsByTarget = new Map<string, BranchInfo[]>();
  for (const b of branches) {
    refsByTarget.set(b.target, [...(refsByTarget.get(b.target) ?? []), b]);
  }
  const alloc = (id: string): number => {
    const free = lanes.indexOf(null);
    if (free >= 0) {
      lanes[free] = id;
      return free;
    }
    lanes.push(id);
    return lanes.length - 1;
  };
  for (const commit of commits) {
    const lanesIn = lanes.slice();
    let lane = lanes.indexOf(commit.id);
    if (lane < 0) lane = alloc(commit.id);
    // Other lanes waiting for this same commit converge here.
    for (let i = 0; i < lanes.length; i++) if (i !== lane && lanes[i] === commit.id) lanes[i] = null;
    const parentLanes: number[] = [];
    const [first, ...rest] = commit.parents;
    if (first !== undefined) {
      const existing = lanes.indexOf(first);
      if (existing >= 0 && existing !== lane) {
        // First parent already expected elsewhere: merge into that lane, free ours.
        lanes[lane] = null;
        parentLanes.push(existing);
      } else {
        lanes[lane] = first;
        parentLanes.push(lane);
      }
    } else {
      lanes[lane] = null;
    }
    for (const p of rest) {
      const existing = lanes.indexOf(p);
      parentLanes.push(existing >= 0 ? existing : alloc(p));
    }
    while (lanes.length && lanes[lanes.length - 1] === null) lanes.pop();
    rows.push({ commit, lane, lanesIn, lanesOut: lanes.slice(), parentLanes, refs: refsByTarget.get(commit.id) ?? [] });
  }
  return rows;
}

export function laneCount(rows: GraphRow[]): number {
  return rows.reduce((m, r) => Math.max(m, r.lanesIn.length, r.lanesOut.length, r.lane + 1, ...r.parentLanes.map((p) => p + 1)), 0);
}
