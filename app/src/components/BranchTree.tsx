import { createMemo, For, Show } from "solid-js";
import type { BranchInfo, CommitInfo } from "../api/types";
import { relativeTime, shortId } from "../lib/format";
import { laneCount, layoutGraph, type GraphRow } from "../lib/graph";
import { Badge } from "./ui";

const ROW = 28;
const LANE = 14;
const COLORS = ["#818cf8", "#34d399", "#f59e0b", "#f472b6", "#38bdf8", "#a78bfa", "#fb7185", "#4ade80"];
const kindTone = { origin: "sky", local: "emerald", agent: "indigo" } as const;

/** Commit graph with lanes, branch badges per commit. Clicking a badge selects that branch. */
export function BranchTree(props: { commits: CommitInfo[]; branches: BranchInfo[]; onSelectBranch?: (name: string) => void }) {
  const rows = createMemo(() => layoutGraph(props.commits, props.branches));
  const width = createMemo(() => Math.max(1, laneCount(rows())) * LANE + LANE);
  return (
    <div class="overflow-x-auto" data-testid="branch-tree">
      <For each={rows()}>
        {(row) => (
          <div class="flex items-center gap-3 text-sm hover:bg-zinc-900/60" style={{ height: `${ROW}px` }} data-testid="graph-row" data-commit={row.commit.id}>
            <svg width={width()} height={ROW} class="shrink-0">
              <RowLines row={row} />
            </svg>
            <div class="flex shrink-0 gap-1">
              <For each={row.refs}>
                {(b) => (
                  <button type="button" onClick={() => props.onSelectBranch?.(b.name)} data-testid={`branch-${b.name}`} title={b.full_ref}>
                    <Badge tone={kindTone[b.kind]}>
                      {b.is_head ? "● " : ""}
                      {b.name}
                    </Badge>
                  </button>
                )}
              </For>
            </div>
            <span class="min-w-0 flex-1 truncate text-zinc-300">{row.commit.summary}</span>
            <span class="shrink-0 font-mono text-xs text-zinc-600">{shortId(row.commit.id)}</span>
            <span class="w-20 shrink-0 text-right text-xs text-zinc-600">{relativeTime(row.commit.time)}</span>
          </div>
        )}
      </For>
      <Show when={!props.commits.length}>
        <p class="p-4 text-sm text-zinc-500">No commits</p>
      </Show>
    </div>
  );
}

function RowLines(props: { row: GraphRow }) {
  const x = (lane: number) => lane * LANE + LANE / 2 + 2;
  const mid = ROW / 2;
  const color = (lane: number) => COLORS[lane % COLORS.length];
  return (
    <>
      <For each={props.row.lanesIn}>
        {(id, i) => (
          <Show when={id}>
            <line
              x1={x(i())}
              y1={0}
              x2={id === props.row.commit.id ? x(props.row.lane) : x(i())}
              y2={mid}
              stroke={color(i())}
              stroke-width="1.5"
            />
          </Show>
        )}
      </For>
      <For each={props.row.lanesOut}>
        {(id, j) => (
          <Show when={id && !props.row.parentLanes.includes(j())}>
            <line x1={x(j())} y1={mid} x2={x(j())} y2={ROW} stroke={color(j())} stroke-width="1.5" />
          </Show>
        )}
      </For>
      <For each={props.row.parentLanes}>
        {(p) => <line x1={x(props.row.lane)} y1={mid} x2={x(p)} y2={ROW} stroke={color(p)} stroke-width="1.5" />}
      </For>
      <circle cx={x(props.row.lane)} cy={mid} r="4" fill={color(props.row.lane)} stroke="#09090b" stroke-width="1.5" />
    </>
  );
}
