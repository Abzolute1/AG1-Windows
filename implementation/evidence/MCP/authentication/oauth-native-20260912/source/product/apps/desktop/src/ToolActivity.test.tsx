import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, expect, test } from "vitest";
import { previewText, ToolOutput } from "./ToolActivity";
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
