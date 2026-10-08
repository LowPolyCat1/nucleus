// Guards the Solid 2 RC behaviours the UI relies on; a failure here after a version bump means
// the framework changed under us, not that the app is broken.
import { render, fireEvent } from "@solidjs/testing-library";
import { createMemo, createSignal, Errored, Loading } from "solid-js";
import { expect, test } from "vitest";

test("async memo refetches on dependency change and errors reach Errored", async () => {
  const [n, setN] = createSignal(1);
  let calls = 0;
  function View() {
    const data = createMemo(async () => {
      const v = n();
      calls++;
      await new Promise((r) => setTimeout(r, 5));
      if (v === 3) throw new Error("three");
      return `value ${v}`;
    });
    return (
      <Errored fallback={(err) => <p>error: {String((err() as Error).message ?? err())}</p>}>
        <Loading fallback={<p>loading</p>}>
          <p>{data()}</p>
        </Loading>
      </Errored>
    );
  }
  const r = render(() => (<div><button onClick={() => setN((x) => x + 1)}>next</button><View /></div>));
  expect(await r.findByText("value 1")).toBeInTheDocument();
  fireEvent.click(r.getByText("next"));
  expect(await r.findByText("value 2")).toBeInTheDocument();
  fireEvent.click(r.getByText("next"));
  expect(await r.findByText(/error: three/)).toBeInTheDocument();
  expect(calls).toBe(3);
});

test("effect apply phase may write signals", async () => {
  const { createEffect } = await import("solid-js");
  const [src, setSrc] = createSignal(1);
  function View() {
    const [out, setOut] = createSignal(0);
    createEffect(() => src(), (v) => { setOut(v * 10); });
    return <p>out {out()}</p>;
  }
  const r = render(() => <View />);
  expect(await r.findByText("out 10")).toBeInTheDocument();
  setSrc(2);
  expect(await r.findByText("out 20")).toBeInTheDocument();
});
