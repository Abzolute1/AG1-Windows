import { afterEach, expect, test, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { TurnOutputs } from "./TurnOutputs";
import type { JsonValue } from "./generated/protocol";
afterEach(cleanup);
const output = { id: "native-output", kind: "terminal-output", bytes: 123, sha256: "a".repeat(64), created_at: "2026-09-10T00:00:00Z" };

test("native output card opens the selected turn's retained artifacts", async () => {
  const review = vi.fn(), run = vi.fn(async () => ({ artifacts: [output], next: null }));
  render(<TurnOutputs turn="one" revision="1" run={run} review={review} />);
  await screen.findByText("1 retained output");
  expect(screen.getByText("terminal output")).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Review turn outputs" }));
  expect(review).toHaveBeenCalledExactlyOnceWith("one");
  expect(run).toHaveBeenCalledWith({ kind: "artifacts", action: "list", args: { turn: "one", before: null } });
});

test("empty native output catalog does not create a claimed output", async () => {
  render(<TurnOutputs turn="one" revision="1" run={async () => ({ artifacts: [], next: null })} review={vi.fn()} />);
  await act(async () => {});
  expect(screen.queryByRole("region", { name: "Retained outputs for this turn" })).toBeNull();
});

test("late output catalogs cannot cross turn selection", async () => {
  let resolve!: (value: JsonValue) => void;
  const view = render(<TurnOutputs turn="one" revision="1" run={() => new Promise(done => { resolve = done; })} review={vi.fn()} />);
  view.rerender(<TurnOutputs turn="two" revision="1" run={async () => ({ artifacts: [], next: null })} review={vi.fn()} />);
  await act(async () => resolve({ artifacts: [output], next: null }));
  expect(screen.queryByText("terminal output")).toBeNull();
});
