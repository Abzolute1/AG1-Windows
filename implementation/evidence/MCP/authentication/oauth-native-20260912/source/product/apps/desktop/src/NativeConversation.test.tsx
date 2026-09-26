import { afterEach, expect, test, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { NativeConversation, TurnView } from "./NativeConversation";
import type { BridgeSnapshot, ClientDetails, ClientMessage, JsonValue } from "./generated/protocol";
afterEach(cleanup);
const user: ClientMessage = { id: "u1", role: "user", blocks: [{ kind: "text", text: "Review this" }] };
test("answer streams before completion, thinking starts collapsed and stays expanded across commit", async () => {
  const run = vi.fn(async () => null);
  const live = { id: "generation-1", committed: false, blocks: [{ kind: "thinking" as const, text: "First thought" }, { kind: "text" as const, text: "Answer begins" }] };
  const { rerender } = render(<TurnView turn={{ id: "u1", messages: [user] }} active live={live} run={run} />);
  expect(screen.getByText("Answer begins")).toBeTruthy(); expect(screen.queryByText("First thought")).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "Thinking" }));
  rerender(<TurnView turn={{ id: "u1", messages: [user] }} active live={{ ...live, blocks: [{ kind: "thinking", text: "First thought, then a second" }, { kind: "text", text: "Answer begins and continues" }] }} run={run} />);
  expect(screen.getByText("First thought, then a second")).toBeTruthy();
  const assistant: ClientMessage = { id: "a1", role: "assistant", blocks: [{ kind: "thinking", text: "First thought, then a second" }, { kind: "text", text: "Complete answer" }] };
  rerender(<TurnView turn={{ id: "u1", messages: [user, assistant] }} active={false} live={{ ...live, committed: true }} run={run} />);
  expect(screen.getByText("First thought, then a second")).toBeTruthy(); expect(screen.getByRole("button", { name: "Thoughts" }).getAttribute("aria-expanded")).toBe("true");
  expect(screen.getByText("Complete answer")).toBeTruthy(); expect(screen.queryByText("Answer begins")).toBeNull();
});
test("journal hydration survives view polling while its request is in flight", async () => {
  let finish!: (value: JsonValue) => void;
  const run = vi.fn(() => new Promise<JsonValue>(resolve => { finish = resolve; }));
  const assistant: ClientMessage = { id: "a1", role: "assistant", blocks: [{ kind: "text", text: "Short [open journal page]" }] };
  const { rerender } = render(<TurnView turn={{ id: "u1", messages: [user, assistant] }} active={false} run={run} />);
  rerender(<TurnView turn={{ id: "u1", messages: [user, { ...assistant }] }} active={false} run={run} />);
  finish({ base64: btoa(JSON.stringify({ message: { content: [{ type: "text", text: "Entire durable answer" }] } })), next: null });
  expect(await screen.findByText("Entire durable answer")).toBeTruthy(); expect(run).toHaveBeenCalledTimes(1);
});
test("long live blocks page while the answer remains visible", async () => {
  const run = vi.fn(async () => ({ base64: btoa("A much longer thought"), offset: 21, total: 21 }));
  const live = { id: "g", committed: false, blocks: [{ kind: "thinking" as const, key: "t", text: "A", bytes: 21, truncated: true }, { kind: "text" as const, key: "a", text: "Visible answer" }] };
  render(<TurnView turn={{ id: "u1", messages: [user] }} active live={live} run={run} />);
  expect(screen.getByText("Visible answer")).toBeTruthy(); fireEvent.click(screen.getByRole("button", { name: "Thinking" }));
  await waitFor(() => expect(screen.getByText("A much longer thought")).toBeTruthy());
  expect(run.mock.calls.length).toBe(1);
});

test("many live tool rounds stay behind a single expandable activity row", () => {
  const work: ClientMessage[] = Array.from({ length: 12 }, (_, i) => ({ id: `tool-${i}`, role: "toolResult", blocks: [{ kind: "text", text: `Tool result number ${i}` }] }));
  render(<TurnView turn={{ id: user.id, messages: [user, ...work] }} active run={vi.fn()} live={{ id: "live", committed: false, blocks: [{ kind: "text", text: "Here is the current answer." }] }} />);
  expect(screen.queryByText("Tool result number 0")).toBeNull();
  expect(screen.getByText("Here is the current answer.")).toBeTruthy();
  expect(screen.getByRole("button", { name: /Working.*12 tools/ }).getAttribute("aria-expanded")).toBe("true");
  expect(screen.getAllByText("Tool output")).toHaveLength(12);
});

function historyFixture(messages: ClientMessage[], count: number) {
  return { details: { messages, activity: {}, task: null, context: {}, memory: null, artifacts: [], models: [], capabilities: [], authority: "rust-isolated" } as ClientDetails, snapshot: { message_count: count, status: "idle" } as BridgeSnapshot };
}
test("loaded history survives the sliding snapshot and missing middle pages are recovered", async () => {
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
  const messages: ClientMessage[] = Array.from({ length: 6 }, (_, i) => ({ id: `history-${i}`, role: i % 2 ? "assistant" : "user", blocks: [{ kind: "text", text: `Message ${i}` }] }));
  const run = vi.fn(async () => ({ messages: messages.slice(2, 4), start: 2, total: 6, next_before: 2 }));
  const { rerender } = render(<NativeConversation {...historyFixture(messages.slice(0, 2), 2)} run={run} />);
  expect(screen.getByText("Message 0")).toBeTruthy();
  rerender(<NativeConversation {...historyFixture(messages.slice(4), 6)} run={run} />);
  await screen.findByText("Message 2");
  expect(screen.getByText("Message 0")).toBeTruthy(); expect(screen.getByText("Message 5")).toBeTruthy();
  expect(run).toHaveBeenCalledWith({ kind: "session", action: "messages", args: { before: 4 } });
});
test("a reopened chat loads its first messages from earlier pages", async () => {
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
  const messages: ClientMessage[] = Array.from({ length: 4 }, (_, i) => ({ id: `reopen-${i}`, role: i % 2 ? "assistant" : "user", blocks: [{ kind: "text", text: `Saved message ${i}` }] }));
  const run = vi.fn(async () => ({ messages: messages.slice(0, 2), start: 0, total: 4, next_before: null }));
  render(<NativeConversation {...historyFixture(messages.slice(2), 4)} run={run} />);
  expect(await screen.findByText("Saved message 0")).toBeTruthy(); expect(screen.queryByText("Load earlier messages")).toBeNull();
});


test("Analyzing image appears during image processing and disappears on first output, loading, cancellation and text-only turns", async () => {
  const { ImageAnalysisStatus } = await import("./NativeConversation");
  const snapshot = { status: "running", pending: [] } as any;
  const details = { messages: [{ id: "image-user", role: "user", blocks: [{ kind: "attachment", media_type: "image/png", reference: "image-user" }] }], activity: { observations: { phase: "processing", first_delta_ms: null }, runtime_preparation: { phase: "ready" } } } as any;
  const view = render(<ImageAnalysisStatus details={details} snapshot={snapshot} />);
  expect(screen.getByText("Analyzing image")).toBeTruthy();
  for (const phase of ["loading", "verifying"]) {
    view.rerender(<ImageAnalysisStatus details={{ ...details, activity: { ...details.activity, runtime_preparation: { phase } } }} snapshot={snapshot} />);
    expect(screen.queryByText("Analyzing image")).toBeNull();
  }
  view.rerender(<ImageAnalysisStatus details={{ ...details, activity: { observations: { phase: "thinking", first_delta_ms: 0 } } }} snapshot={snapshot} />);
  expect(screen.queryByText("Analyzing image")).toBeNull();
  view.rerender(<ImageAnalysisStatus details={details} snapshot={{ ...snapshot, status: "idle" }} />);
  expect(screen.queryByText("Analyzing image")).toBeNull();
  view.rerender(<ImageAnalysisStatus details={{ ...details, messages: [user] }} snapshot={snapshot} />);
  expect(screen.queryByText("Analyzing image")).toBeNull();
});

test("progress stays visible, reversed tool results pair by call identity, and source stays literal", () => {
  const assistant: ClientMessage = { id: "audit", role: "assistant", blocks: [
    { kind: "text", text: "Checking the entry point and error handling." },
    { kind: "tool", call_id: "read-html", name: "read", arguments: { path: "index.html" } },
    { kind: "tool", call_id: "read-css", name: "read", arguments: { path: "style.css" } },
  ] };
  const html = '<!DOCTYPE html>\n<html lang="en">\n    <script>alert("never execute")</script>\n</html>';
  const css: ClientMessage = { id: "css-result", role: "toolResult", tool: { call_id: "read-css", name: "read", failed: false }, blocks: [{ kind: "text", text: "body { color: red; }" }] };
  const result: ClientMessage = { id: "html-result", role: "toolResult", tool: { call_id: "read-html", name: "read", failed: false }, blocks: [{ kind: "text", text: html }] };
  const { container } = render(<TurnView turn={{ id: "u1", messages: [user, assistant, css, result] }} active run={vi.fn()} />);
  expect(screen.getByText("Checking the entry point and error handling.")).toBeTruthy();
  expect(container.querySelector(".tool-output-text")).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: /Read index.html/ }));
  expect(screen.getByRole("region", { name: "Tool output" }).textContent).toBe(html);
  expect(screen.queryByText("body { color: red; }")).toBeNull();
  expect(container.querySelector("script")).toBeNull();
});

test("collapsed large results do not load journal payloads until the paired entry is expanded", async () => {
  const assistant: ClientMessage = { id: "call", role: "assistant", blocks: [{ kind: "tool", name: "read", call_id: "c1", arguments: { path: "large.ts" } }] };
  const result: ClientMessage = { id: "r1", role: "toolResult", tool: { name: "read", call_id: "c1", failed: false }, blocks: [{ kind: "text", text: "short [open journal page]" }] };
  const run = vi.fn(async () => ({ base64: btoa(JSON.stringify({ message: { content: "Complete literal source" } })), next: null }));
  render(<TurnView turn={{ id: "u1", messages: [user, assistant, result] }} active={false} run={run} />);
  expect(run).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: /Work details 1 tool/ }));
  expect(run).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: /Read large.ts/ }));
  expect(await screen.findByText("Complete literal source")).toBeTruthy();
  expect(run).toHaveBeenCalledTimes(1);
});

test("multiple model rounds share one work header, keep chronological progress and tuck read-only GitHub cards away", () => {
  const first: ClientMessage = { id: "a1", role: "assistant", blocks: [{ kind: "text", text: "Checking access." }, { kind: "tool", call_id: "g1", name: "github", arguments: { operation: "repos.list" } }] };
  const result: ClientMessage = { id: "r1", role: "toolResult", tool: { call_id: "g1", name: "github", failed: false }, blocks: [{ kind: "text", text: '{"status":200,"data":[]}' }] };
  const second: ClientMessage = { id: "a2", role: "assistant", blocks: [{ kind: "text", text: "Inspecting source next." }, { kind: "tool", call_id: "s1", name: "inspect", arguments: { path: "src/main.rs" } }] };
  const source: ClientMessage = { id: "r2", role: "toolResult", tool: { call_id: "s1", name: "inspect", failed: false }, blocks: [{ kind: "text", text: "SOURCE" }] };
  const messages = [user, first, result, second, source];
  const view = render(<TurnView turn={{ id: user.id, messages }} active workTime="12s" run={vi.fn()} />);
  expect(document.querySelectorAll('.work-toggle')).toHaveLength(1);
  expect(screen.getByRole("button", { name: /Working for 12s/ })).toBeTruthy();
  expect(screen.queryByRole("region", { name: "GitHub result" })).toBeNull();
  expect(document.querySelector('.work-log')?.textContent).toMatch(/Checking access.*Listed repositories.*Inspecting source next.*Read file/s);
  view.rerender(<TurnView turn={{ id: user.id, messages: [...messages, { id: "final", role: "assistant", blocks: [{ kind: "text", text: "The check passed." }] }] }} active={false} workTime="14s" run={vi.fn()} />);
  expect(screen.getByText("The check passed.")).toBeTruthy();
  expect(screen.queryByText("Checking access.")).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: /Worked for 14s/ }));
  fireEvent.click(screen.getByRole("button", { name: /Listed repositories/ }));
  expect(screen.getByRole("region", { name: "Tool output" }).textContent).toContain('"status":200');
  expect(screen.queryByRole("button", { name: "Inspect GitHub result" })).toBeNull();
});

test("collapsed work advertises failures and preserves uncertain GitHub receipts", () => {
  const messages: ClientMessage[] = [user, { id: "a", role: "assistant", blocks: [{ kind: "tool", name: "github", call_id: "g", arguments: { operation: "issues.create" } }] }, { id: "r", role: "toolResult", tool: { name: "github", call_id: "g", failed: true }, blocks: [{ kind: "text", text: '{"outcome":"unknown","message":"Interrupted"}' }] }];
  render(<TurnView turn={{ id: user.id, messages }} active={false} run={vi.fn()} />);
  expect(screen.getByRole("button", { name: /Work details 1 tool 1 failed/ })).toBeTruthy();
  expect(screen.getByText("GitHub outcome uncertain")).toBeTruthy();
});

test("a terminal provider error overrides stale thinking activity and remains visible", () => {
  const failed: ClientMessage = { id: "failed", role: "assistant", blocks: [{ kind: "unknown", value: { kind: "provider_error", message: "Unsupported stream event", cancelled: false } }, { kind: "thinking", text: "" }] };
  render(<TurnView turn={{ id: user.id, messages: [user, failed] }} active live={{ id: "stale", committed: false, blocks: [{ kind: "thinking", text: "" }] }} run={vi.fn()} />);
  expect(screen.getByRole("alert").textContent).toContain("Response failed");
  expect(screen.queryByRole("button", { name: "Thinking" })).toBeNull();
  expect(screen.queryByRole("button", { name: "Thoughts" })).toBeNull();
});

test("startup progress appears before assistant output and updates through waiting, thinking and stop", () => {
  const props = { turn: { id: user.id, messages: [user] }, run: vi.fn(), workTime: "12s" };
  const view = render(<TurnView {...props} active workStatus="Loading Qwen model" />);
  expect(screen.getByRole("button", { name: "Working for 12s" }).hasAttribute("disabled")).toBe(true);
  expect(screen.getByRole("status").textContent).toBe("Loading Qwen model");
  expect(screen.queryByText("0 tools")).toBeNull();
  for (const status of ["Preparing request", "Waiting for model", "Thinking", "Stopping response…"]) {
    view.rerender(<TurnView {...props} active workStatus={status} />);
    expect(screen.getByRole("status").textContent).toBe(status);
  }
  view.rerender(<TurnView {...props} active={false} workStatus="Thinking" />);
  expect(screen.queryByRole("status")).toBeNull();
  expect(screen.queryByText("Working for 12s")).toBeNull();
});
