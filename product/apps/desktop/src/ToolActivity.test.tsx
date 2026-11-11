import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, expect, test } from "vitest";
import { ActivityGroup, previewText, ToolOutput } from "./ToolActivity";
afterEach(cleanup);
test("native preview hunks decode as literal diff with a bounded raw fallback", () => {
  const text = JSON.stringify({ path: "λ.html", hunks: [{ fromLine: 1, toLine: 1, diff: '@@ -1 +1 @@\n-old\n+<script>alert("x")</script>' }] });
  expect(previewText(text)).toContain('\n+<script>alert("x")</script>');
  expect(previewText('{"path":')).toBeNull();
  expect(previewText(JSON.stringify({ path: "a", hunks: [{ diff: 1 }] }))).toBeNull();
  expect(previewText("x".repeat(256 * 1024 + 1))).toBeNull();
  render(<ToolOutput message={{ id: "r", role: "toolResult", blocks: [{ kind: "text", text }], tool: { name: "editPreview", call_id: "c", failed: false } } as any} run={async () => null} />);
  expect(screen.getByText("Preview")).toBeTruthy();
  expect(screen.getByRole("region", { name: "Tool output" }).textContent).toContain("+<script>");
  expect(document.querySelector("script")).toBeNull();
});

test("blocked web searches read as a recoverable availability problem", () => {
  const messages = [
    { id: "a", role: "assistant", blocks: [{ kind: "tool", name: "webSearch", call_id: "s", arguments: { query: "Rust documentation" } }] },
    { id: "r", role: "toolResult", tool: { name: "webSearch", call_id: "s", failed: true }, blocks: [{ kind: "text", text: "DuckDuckGo requires human verification." }] },
  ];
  render(<ActivityGroup messages={messages as any} active={false} compact={false} run={async () => null} onExpand={() => {}} loading={() => false} />);
  expect(screen.getByRole("button", { name: /Search unavailable.*Unavailable/ })).toBeTruthy();
  expect(screen.queryByText("Searching the web")).toBeNull();
  expect(screen.queryByText("Searched the web")).toBeNull();
  expect(screen.getByText("1 recoverable outcome")).toBeTruthy();
  expect(screen.queryByText("1 failed")).toBeNull();
});

test("ordinary tool faults remain visibly failed", () => {
  const messages = [
    { id: "a", role: "assistant", blocks: [{ kind: "tool", name: "bash", call_id: "b", arguments: { command: "build" } }] },
    { id: "r", role: "toolResult", tool: { name: "bash", call_id: "b", failed: true }, blocks: [{ kind: "text", text: "Process exited with status 1." }] },
  ];
  render(<ActivityGroup messages={messages as any} active={false} compact={false} run={async () => null} onExpand={() => {}} loading={() => false} />);
  expect(screen.getByRole("button", { name: /Running command.*Failed/ })).toBeTruthy();
  expect(screen.getByText("1 failed")).toBeTruthy();
});

test("recoverable command outcomes are not counted as failures", () => {
  const messages = [
    { id: "path", role: "assistant", blocks: [{ kind: "tool", name: "bash", call_id: "path", arguments: { command: "git clone https://example.test/repo" } }] },
    { id: "path-result", role: "toolResult", tool: { name: "bash", call_id: "path", failed: true }, blocks: [{ kind: "text", text: "fatal: could not create work tree dir 'repo': Read-only file system" }] },
    { id: "node", role: "assistant", blocks: [{ kind: "tool", name: "bash", call_id: "node", arguments: { command: "npm install" } }] },
    { id: "node-result", role: "toolResult", tool: { name: "bash", call_id: "node", failed: true }, blocks: [{ kind: "text", text: "npm error code EBADENGINE: Unsupported engine" }] },
  ];
  render(<ActivityGroup messages={messages as any} active={false} compact={false} run={async () => null} onExpand={() => {}} loading={() => false} />);
  expect(screen.getByRole("button", { name: /Ran command.*Path not writable/ })).toBeTruthy();
  expect(screen.getByRole("button", { name: /Ran command.*Runtime prerequisite missing/ })).toBeTruthy();
  expect(screen.getByText("2 recoverable outcomes")).toBeTruthy();
  expect(screen.queryByText("2 failed")).toBeNull();
});

test("expected empty shell searches are muted rather than failed", () => {
  const messages = [
    { id: "a", role: "assistant", blocks: [{ kind: "tool", name: "bash", call_id: "grep", arguments: { command: "grep -i needle files" } }] },
    { id: "r", role: "toolResult", tool: { name: "bash", call_id: "grep", failed: true }, blocks: [{ kind: "text", text: "(no output)\n\nCommand exited with code 1" }] },
  ];
  render(<ActivityGroup messages={messages as any} active={false} compact={false} run={async () => null} onExpand={() => {}} loading={() => false} />);
  expect(screen.getByRole("button", { name: /Ran command.*No matching result/ })).toBeTruthy();
  expect(screen.getByText("1 recoverable outcome")).toBeTruthy();
  expect(screen.queryByText("1 failed")).toBeNull();
});

test("repeated DuckDuckGo blocks are summarized without concealing their rows", () => {
  const messages = ["one", "two"].flatMap((query, index) => [
    { id: `a${index}`, role: "assistant", blocks: [{ kind: "tool", name: "webSearch", call_id: `s${index}`, arguments: { query } }] },
    { id: `r${index}`, role: "toolResult", tool: { name: "webSearch", call_id: `s${index}`, failed: true }, blocks: [{ kind: "text", text: "DuckDuckGo web blocked" }] },
  ]);
  render(<ActivityGroup messages={messages as any} active={false} compact={false} run={async () => null} onExpand={() => {}} loading={() => false} />);
  expect(screen.getByText("DuckDuckGo unavailable (2 searches paused)")).toBeTruthy();
  expect(screen.getAllByText("Search unavailable")).toHaveLength(2);
});
