import { expect, test } from "vitest";
import { liveToolDisplay } from "./LiveBlocks";

test("live web calls show a useful state instead of raw empty arguments", () => {
  expect(liveToolDisplay({ kind: "tool", name: "webSearch", call_id: "search", arguments: {} } as any)).toEqual({ label: "Preparing web search", target: "", kind: "search" });
  expect(liveToolDisplay({ kind: "tool", name: "webSearch", call_id: "search", arguments: { query: "Qwen 4 release" } } as any)).toEqual({ label: "Searching the web", target: "Qwen 4 release", kind: "search" });
  expect(liveToolDisplay({ kind: "tool", name: "webFetch", call_id: "page", arguments: { url: "https://example.com" } } as any)).toEqual({ label: "Reading webpage", target: "https://example.com", kind: "page" });
  expect(liveToolDisplay({ kind: "tool", name: "mcpSearch", call_id: "catalog", arguments: { query: "repository" } } as any)).toEqual({ label: "Searching connected tools", target: "repository", kind: "search" });
  expect(liveToolDisplay({ kind: "tool", name: "mcpCall", call_id: "call", arguments: { name: "get_status" } } as any)).toEqual({ label: "Running connected tool", target: "get_status", kind: "tool" });
});
