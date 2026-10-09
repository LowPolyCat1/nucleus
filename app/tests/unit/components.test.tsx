import { fireEvent, render, waitFor, within } from "@solidjs/testing-library";
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

import { createSignal } from "solid-js";
import { StreamedDiff } from "../../src/components/DiffView";

function controlledLoad() {
  let emit: (f: FileDiff) => void = () => {};
  let done: (n: number) => void = () => {};
  let fail: (e: unknown) => void = () => {};
  const load = (onFile: (f: FileDiff) => void) =>
    new Promise<number>((res, rej) => {
      emit = onFile;
      done = res;
      fail = rej;
    });
  return { load, emit: (f: FileDiff) => emit(f), done: (n: number) => done(n), fail: (e: unknown) => fail(e) };
}

test("StreamedDiff renders files as they arrive", async () => {
  const c = controlledLoad();
  const r = render(() => <StreamedDiff load={c.load} streamKey="k" />);
  expect(r.getByTestId("diff-loading")).toHaveTextContent("0 files so far");
  c.emit(file({ path: "one.txt" }));
  expect(await r.findByTestId("diff-file-one.txt")).toBeInTheDocument();
  expect(r.getByTestId("diff-loading")).toHaveTextContent("1 file so far");
  c.emit(file({ path: "two.txt" }));
  c.done(2);
  await waitFor(() => expect(r.getByTestId("streamed-diff")).toHaveAttribute("data-loading", "false"));
  expect(r.getByTestId("diff-file-two.txt")).toBeInTheDocument();
  expect(r.queryByTestId("diff-loading")).not.toBeInTheDocument();
});

test("StreamedDiff drops superseded streams and shows errors", async () => {
  const first = controlledLoad();
  const second = controlledLoad();
  const [key, setKey] = createSignal("a");
  const r = render(() => <StreamedDiff load={(f) => (key() === "a" ? first.load(f) : second.load(f))} streamKey={key()} emptyText="Same" />);
  first.emit(file({ path: "old.txt" }));
  await r.findByTestId("diff-file-old.txt");
  setKey("b");
  await waitFor(() => expect(r.queryByTestId("diff-file-old.txt")).not.toBeInTheDocument());
  first.emit(file({ path: "late.txt" }));
  first.done(2);
  second.done(0);
  expect(await r.findByText("Same")).toBeInTheDocument();
  expect(r.queryByTestId("diff-file-late.txt")).not.toBeInTheDocument();
  setKey("a");
  flush(); // let the effect start the new stream before failing it
  first.fail("diff failed: bad object");
  expect(await r.findByText("diff failed: bad object")).toBeInTheDocument();
});

test("truncated files say so", () => {
  const r = render(() => <DiffView files={[file({ path: "big.txt", truncated: true }), file({ path: "huge.bin", truncated: true, patch: "" })]} />);
  expect(within(r.getByTestId("diff-file-big.txt")).getByTestId("truncated")).toHaveTextContent("Diff truncated");
  expect(within(r.getByTestId("diff-file-huge.bin")).getByTestId("truncated")).toHaveTextContent("too large to diff");
});
