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

test("blocked web searches stop looking active and expose the failure", () => {
  const messages = [
    { id: "a", role: "assistant", blocks: [{ kind: "tool", name: "webSearch", call_id: "s", arguments: { query: "Rust documentation" } }] },
    { id: "r", role: "toolResult", tool: { name: "webSearch", call_id: "s", failed: true }, blocks: [{ kind: "text", text: "DuckDuckGo requires human verification." }] },
  ];
  render(<ActivityGroup messages={messages as any} active={false} compact={false} run={async () => null} onExpand={() => {}} loading={() => false} />);
  expect(screen.getByRole("button", { name: /Search unavailable.*Failed/ })).toBeTruthy();
  expect(screen.queryByText("Searching the web")).toBeNull();
  expect(screen.queryByText("Searched the web")).toBeNull();
  expect(screen.getByText("1 failed")).toBeTruthy();
});
