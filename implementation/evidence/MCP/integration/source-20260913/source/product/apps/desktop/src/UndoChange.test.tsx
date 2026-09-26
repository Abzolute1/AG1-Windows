import { afterEach, expect, test, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { UndoChange } from "./UndoChange";
import type { JsonValue } from "./generated/protocol";
afterEach(cleanup);

test("Undo requires an explicit confirmation and uses the native effect channel", async () => {
  let done = false;
  const run = vi.fn(async (): Promise<JsonValue> => done ? { allowed: false, outcome: { state: "completed", recovery: ".ag1-undo-owned" } } : { allowed: true, operation: "restore_original" });
  const effect = vi.fn(async () => { done = true; return { state: "completed" }; });
  render(<UndoChange id="native-receipt" path="notes.txt" run={run} effect={effect} />);
  fireEvent.click(await screen.findByRole("button", { name: "Undo file change" }));
  expect(effect).not.toHaveBeenCalled();
  const confirm = screen.getByRole("button", { name: "Undo this file change" });
  expect(fireEvent.keyDown(confirm, { key: "Enter" })).toBe(false);
  expect(effect).not.toHaveBeenCalled();
  fireEvent.click(confirm);
  await screen.findByText("Undo completed.");
  expect(effect).toHaveBeenCalledExactlyOnceWith({ kind: "files", action: "change_undo", args: { id: "native-receipt" } });
  expect(run.mock.calls.length).toBe(2);
});

test("later changes and revoked grants have no Undo action", async () => {
  const effect = vi.fn();
  render(<UndoChange id="native-receipt" path="notes.txt" run={async () => ({ allowed: false, reason: "This file changed after the recorded operation." })} effect={effect} />);
  await screen.findByText("This file changed after the recorded operation.");
  expect(screen.queryByRole("button", { name: "Undo file change" })).toBeNull();
  expect(effect).not.toHaveBeenCalled();
});

test("uncertain effects refresh native status without replay, and expose explicit recovery", async () => {
  let attempted = false;
  const run = async (): Promise<JsonValue> => attempted ? { allowed: false, recoverable: true, outcome: { state: "prepared", recovery: ".ag1-undo-owned" } } : { allowed: true };
  const effect = vi.fn(async () => { attempted = true; throw new Error("Lost acknowledgement"); });
  render(<UndoChange id="native-receipt" path="notes.txt" run={run} effect={effect} />);
  fireEvent.click(await screen.findByRole("button", { name: "Undo file change" }));
  fireEvent.click(screen.getByRole("button", { name: "Undo this file change" }));
  await screen.findByRole("button", { name: "Recover displaced file" });
  expect(effect).toHaveBeenCalledTimes(1);
  expect(screen.getByText(/Undo needs recovery review/)).toBeTruthy();
});

test("switching the selected receipt discards delayed Undo availability", async () => {
  let resolve!: (value: JsonValue) => void;
  const view = render(<UndoChange id="old" path="old.txt" run={() => new Promise(done => { resolve = done; })} effect={vi.fn()} />);
  view.rerender(<UndoChange id="new" path="new.txt" run={async () => ({ allowed: false, reason: "Read-only folder" })} effect={vi.fn()} />);
  await screen.findByText("Read-only folder");
  await act(async () => resolve({ allowed: true }));
  expect(screen.queryByRole("button", { name: "Undo file change" })).toBeNull();
});
