import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { webcrypto } from "node:crypto";
import { TurnChangeCard, TurnReview, type Change } from "./TurnChanges";
import type { JsonValue } from "./generated/protocol";

beforeEach(() => {
  vi.stubGlobal("crypto", webcrypto);
  URL.createObjectURL = vi.fn(() => "blob:native-change"); URL.revokeObjectURL = vi.fn();
});
afterEach(() => { cleanup(); vi.unstubAllGlobals(); });
const change: Change = { id: "change-native", path: "notes.txt", root: "/owned/project", call_id: "call-1", state: "committed", operation: "edit", before_sha256: "a".repeat(64), after_sha256: "b".repeat(64), summary: { addedLines: 1, removedLines: 1 } };
const list = (changes = [change], turn = "turn-1") => ({ turn, changes, next: null, notice: "Native write/edit receipts only." });
async function source(text: string, side: string, id = change.id) {
  const bytes = new TextEncoder().encode(text), sha256 = Array.from(new Uint8Array(await crypto.subtle.digest("SHA-256", bytes)), b => b.toString(16).padStart(2, "0")).join("");
  return { base64: btoa(text), bytes: bytes.length, sha256, next: null, side, id };
}

test("turn card uses native counts and opens its exact turn", async () => {
  const review = vi.fn(), run = vi.fn(async () => list() as any);
  render(<TurnChangeCard turn="turn-1" revision="1" run={run} review={review} />);
  await screen.findByText("1 file changed");
  expect(screen.getByText("+1 −1")).toBeTruthy();
  expect(run).toHaveBeenCalledWith({ kind: "files", action: "changes", args: { turn: "turn-1", offset: 0 } });
  fireEvent.click(screen.getByRole("button", { name: "Review turn changes" }));
  expect(review).toHaveBeenCalledWith("turn-1");
  expect(screen.queryByRole("button", { name: /Undo/ })).toBeNull();
});

test("no native receipts means no edit card, and uncertain effects never count as completed", async () => {
  const view = render(<TurnChangeCard turn="turn-1" revision="1" run={async () => list([]) as any} review={() => {}} />);
  await act(async () => {});
  expect(screen.queryByRole("region", { name: "Native changes for this turn" })).toBeNull();
  view.rerender(<TurnChangeCard turn="turn-1" revision="2" run={async () => list([{ ...change, state: "uncertain" }]) as any} review={() => {}} />);
  await screen.findByText("1 operation needs outcome review.");
  expect(screen.getByText("0 files changed")).toBeTruthy();
  expect(screen.queryByText("+1 −1")).toBeNull();
});

test("turn switches discard delayed native receipt lists", async () => {
  let resolve!: (v: JsonValue) => void;
  const old = () => new Promise<JsonValue>(done => { resolve = done; });
  const view = render(<TurnChangeCard turn="turn-1" revision="1" run={old} review={() => {}} />);
  view.rerender(<TurnChangeCard turn="turn-2" revision="1" run={async () => list([], "turn-2") as any} review={() => {}} />);
  await act(async () => resolve(list() as any));
  expect(screen.queryByText("notes.txt")).toBeNull();
});

test("before and after source render literally and downloads verify exact bytes", async () => {
  const before = await source("User dirty bytes\r\n", "before"), after = await source("<script>literal</script>\n", "after"), diff = await source("-1 User dirty bytes\n+1 <script>literal</script>", "diff");
  const run = vi.fn(async (c: any): Promise<JsonValue> => c.action === "changes" ? list() as any : c.args.side === "before" ? before : c.args.side === "after" ? after : diff);
  const view = render(<TurnReview turn="turn-1" run={run} />);
  fireEvent.click(await screen.findByRole("button", { name: /notes.txt/ }));
  await screen.findByText(/-1 User dirty bytes/);
  fireEvent.click(screen.getByRole("button", { name: "After" }));
  await screen.findByText("<script>literal</script>");
  expect(document.querySelector("script, iframe")).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "Prepare after download" }));
  await screen.findByRole("link", { name: "Download exact after" });
  const blob = vi.mocked(URL.createObjectURL).mock.calls[0][0] as Blob;
  expect(blob.size).toBe(after.bytes); expect(blob.type).toBe("application/octet-stream");
  fireEvent.click(screen.getByRole("button", { name: "Before" }));
  await screen.findByText("User dirty bytes");
  expect(URL.revokeObjectURL).toHaveBeenCalledWith("blob:native-change");
  view.unmount();
});

test("corrupt native snapshot downloads cannot produce links", async () => {
  const bytes = { ...await source("changed", "after"), sha256: "0".repeat(64) };
  const run = async (c: any): Promise<JsonValue> => c.action === "changes" ? list() as any : c.args.side === "diff" ? { unavailable: "Binary change" } : bytes;
  render(<TurnReview turn="turn-1" run={run} />);
  fireEvent.click(await screen.findByRole("button", { name: /notes.txt/ }));
  await screen.findByText("Binary change");
  fireEvent.click(screen.getByRole("button", { name: "After" }));
  await screen.findByText("changed");
  fireEvent.click(screen.getByRole("button", { name: "Prepare after download" }));
  await screen.findByRole("alert");
  expect(URL.createObjectURL).not.toHaveBeenCalled();
  expect(screen.queryByRole("link")).toBeNull();
});

test("changing source selection while a read is pending discards the old page", async () => {
  let resolve!: (v: JsonValue) => void;
  const after = await source("current after", "after");
  const run = async (c: any): Promise<JsonValue> => c.action === "changes" ? list() as any : c.args.side === "after" ? after : new Promise(done => { resolve = done; });
  render(<TurnReview turn="turn-1" run={run} />);
  fireEvent.click(await screen.findByRole("button", { name: /notes.txt/ }));
  fireEvent.click(screen.getByRole("button", { name: "After" }));
  await screen.findByText("current after");
  await act(async () => resolve(await source("old diff", "diff")));
  expect(screen.queryByText("old diff")).toBeNull();
});
