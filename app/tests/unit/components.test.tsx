import { fireEvent, render } from "@solidjs/testing-library";
import { flush } from "solid-js";
import { expect, test } from "vitest";
import type { FileDiff } from "../../src/api/types";
import { BranchTree } from "../../src/components/BranchTree";
import { DiffView } from "../../src/components/DiffView";

const file = (over: Partial<FileDiff>): FileDiff => ({ path: "a.txt", old_path: null, status: "modified", binary: false, additions: 1, deletions: 1, patch: "@@ -1 +1 @@\n-a\n+b\n", ...over });

test("DiffView renders hunks, binary files, renames and toggles", () => {
  const r = render(() => (
    <DiffView files={[file({}), file({ path: "img.png", binary: true, patch: "" }), file({ path: "new.txt", old_path: "old.txt", status: "renamed" })]} />
  ));
  expect(r.getByTestId("diff-summary")).toHaveTextContent("3 files changed, +3 −3");
  const a = r.getByTestId("diff-file-a.txt");
  expect(a.querySelectorAll('tr[data-kind="add"]')).toHaveLength(1);
  expect(a.querySelectorAll('tr[data-kind="del"]')).toHaveLength(1);
  expect(r.getByTestId("diff-file-img.png")).toHaveTextContent("Binary file not shown");
  expect(r.getByTestId("diff-file-new.txt")).toHaveTextContent("old.txt →");
  fireEvent.click(a.querySelector("button")!);
  flush();
  expect(a.querySelectorAll("tr")).toHaveLength(0);
});

test("DiffView empty state and singular summary", () => {
  expect(render(() => <DiffView files={[]} emptyText="Nothing here" />).getByText("Nothing here")).toBeInTheDocument();
  expect(render(() => <DiffView files={[file({})]} />).getByTestId("diff-summary")).toHaveTextContent("1 file changed");
});

test("DiffView collapses files beyond the first 20", () => {
  const many = Array.from({ length: 25 }, (_, i) => file({ path: `f${i}.txt` }));
  const r = render(() => <DiffView files={many} />);
  expect(r.getByTestId("diff-file-f0.txt").querySelectorAll("tr").length).toBeGreaterThan(0);
  expect(r.getByTestId("diff-file-f24.txt").querySelectorAll("tr")).toHaveLength(0);
});

test("BranchTree rows, badges and selection", () => {
  const picked: string[] = [];
  const r = render(() => (
    <BranchTree
      commits={[
        { id: "m", parents: ["b", "x"], summary: "merge", message: "", author_name: "", author_email: "", time: 0 },
        { id: "x", parents: ["a"], summary: "agent", message: "", author_name: "", author_email: "", time: 0 },
        { id: "b", parents: ["a"], summary: "main", message: "", author_name: "", author_email: "", time: 0 },
        { id: "a", parents: [], summary: "root", message: "", author_name: "", author_email: "", time: 0 },
      ]}
      branches={[{ name: "main", full_ref: "refs/heads/main", kind: "local", target: "m", is_head: true }]}
      onSelectBranch={(n) => picked.push(n)}
    />
  ));
  expect(r.getAllByTestId("graph-row")).toHaveLength(4);
  expect(r.getByTestId("branch-main")).toHaveTextContent("● main");
  fireEvent.click(r.getByTestId("branch-main"));
  flush();
  expect(picked).toEqual(["main"]);
  expect(r.container.querySelectorAll("circle")).toHaveLength(4);
});

test("BranchTree empty", () => {
  expect(render(() => <BranchTree commits={[]} branches={[]} />).getByText("No commits")).toBeInTheDocument();
});
