import { expect, test } from "vitest";
import { diffStats, parsePatch } from "../../src/lib/diff";

test("parses hunks with line numbers", () => {
  const hunks = parsePatch("@@ -1,3 +1,3 @@\n one\n-two\n+TWO\n three\n@@ -10 +10,2 @@ fn x\n ten\n+eleven\n");
  expect(hunks).toHaveLength(2);
  expect(hunks[0].lines.map((l) => [l.kind, l.oldNo, l.newNo, l.text])).toEqual([
    ["ctx", 1, 1, "one"],
    ["del", 2, null, "two"],
    ["add", null, 2, "TWO"],
    ["ctx", 3, 3, "three"],
  ]);
  expect(hunks[1].oldStart).toBe(10);
  expect(hunks[1].header).toContain("fn x");
  expect(hunks[1].lines[1]).toEqual({ kind: "add", text: "eleven", oldNo: null, newNo: 11 });
});

test("ignores file headers and junk before the first hunk", () => {
  const hunks = parsePatch("--- a/x\n+++ b/x\nrandom\n@@ -0,0 +1 @@\n+new\n");
  expect(hunks).toHaveLength(1);
  expect(hunks[0].lines).toEqual([{ kind: "add", text: "new", oldNo: null, newNo: 1 }]);
});

test("no newline marker and empty context lines", () => {
  const hunks = parsePatch("@@ -1,2 +1,2 @@\n-a\n\\ No newline at end of file\n+b\n\\ No newline at end of file\n");
  expect(hunks[0].lines.map((l) => l.kind)).toEqual(["del", "meta", "add", "meta"]);
  const blank = parsePatch("@@ -1,3 +1,3 @@\n a\n\n c\n");
  expect(blank[0].lines.map((l) => [l.kind, l.text])).toEqual([["ctx", "a"], ["ctx", ""], ["ctx", "c"]]);
});

test("empty and malformed input", () => {
  expect(parsePatch("")).toEqual([]);
  expect(parsePatch("@@ broken @@\n+x")).toEqual([]);
});

test("stats", () => {
  expect(diffStats([])).toEqual({ additions: 0, deletions: 0 });
  expect(diffStats([{ additions: 2, deletions: 1 }, { additions: 0, deletions: 3 }])).toEqual({ additions: 2, deletions: 4 });
});
