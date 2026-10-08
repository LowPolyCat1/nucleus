import { expect, test } from "vitest";
import type { BranchInfo, CommitInfo } from "../../src/api/types";
import { laneCount, layoutGraph } from "../../src/lib/graph";

const c = (id: string, parents: string[] = []): CommitInfo => ({ id, parents, summary: id, message: id, author_name: "a", author_email: "a@a", time: 0 });
const b = (name: string, target: string, kind: BranchInfo["kind"] = "local"): BranchInfo => ({ name, target, kind, full_ref: `refs/heads/${name}`, is_head: false });

test("empty", () => {
  expect(layoutGraph([])).toEqual([]);
  expect(laneCount([])).toBe(0);
});

test("linear history stays in one lane", () => {
  const rows = layoutGraph([c("c", ["b"]), c("b", ["a"]), c("a")]);
  expect(rows.map((r) => r.lane)).toEqual([0, 0, 0]);
  expect(rows[2].lanesOut).toEqual([]);
  expect(laneCount(rows)).toBe(1);
});

test("branch and merge", () => {
  // m merges x (agent) into main: m -> [b, x], x -> a, b -> a
  const rows = layoutGraph([c("m", ["b", "x"]), c("x", ["a"]), c("b", ["a"]), c("a")], [b("main", "m"), b("agent/1", "x", "agent")]);
  expect(rows[0].parentLanes).toEqual([0, 1]);
  expect(rows[0].refs.map((r) => r.name)).toEqual(["main"]);
  expect(rows[1].lane).toBe(1);
  expect(rows[1].refs[0].kind).toBe("agent");
  // x's parent a is not yet expected; lane 1 now expects a, b keeps lane 0 expecting a too.
  expect(rows[2].lane).toBe(0);
  expect(rows[2].parentLanes).toEqual([1]);
  // a converges both lanes.
  expect(rows[3].lane).toBe(1);
  expect(rows[3].lanesOut).toEqual([]);
  expect(laneCount(rows)).toBe(2);
});

test("two tips on separate lanes and freed lanes are reused", () => {
  const rows = layoutGraph([c("t1", ["a"]), c("t2", ["z"]), c("z"), c("t3", ["a"]), c("a")]);
  expect(rows.map((r) => r.lane)).toEqual([0, 1, 1, 1, 0]);
});

test("parents cut off by the limit keep lanes open", () => {
  const rows = layoutGraph([c("y", ["x"])]);
  expect(rows[0].lanesOut).toEqual(["x"]);
});

test("several branches on one commit", () => {
  const rows = layoutGraph([c("a")], [b("main", "a"), b("origin/main", "a", "origin")]);
  expect(rows[0].refs).toHaveLength(2);
});

test("octopus merge allocates a lane per extra parent", () => {
  const rows = layoutGraph([c("m", ["p1", "p2", "p3"]), c("p3"), c("p2"), c("p1")]);
  expect(rows[0].parentLanes).toEqual([0, 1, 2]);
  expect(laneCount(rows)).toBe(3);
});
