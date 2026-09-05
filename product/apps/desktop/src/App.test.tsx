import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { render, screen, fireEvent, cleanup, waitFor } from "@testing-library/react";
import App from "./App";
import { previewDocument } from "./preview";
vi.mock("@tauri-apps/api/event", () => ({ listen: async () => () => {} }));
const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
beforeEach(() => {
  vi.stubGlobal("matchMedia", () => ({ matches: false }));
  invoke.mockImplementation(async (name: string) =>
    name === "catalog"
      ? { projects: [], theme: "light" }
      : { snapshot: null, error: null, progress: "" },
  );
});
afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});
test("home opens a keyboard-accessible project dialog without starting a model", async () => {
  render(<App />);
  await screen.findByText("What will you work on?");
  fireEvent.keyDown(document, { key: "o", ctrlKey: true });
  expect(await screen.findByRole("dialog")).toBeTruthy();
  expect(screen.getByRole("textbox", { name: "Project folder" })).toBeTruthy();
  expect(invoke.mock.calls.some(([name]) => name === "bridge_command")).toBe(false);
  fireEvent.click(screen.getByText("Cancel"));
  await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
});
test("theme choice uses host preferences", async () => {
  render(<App />);
  await screen.findByText("What will you work on?");
  fireEvent.click(screen.getByRole("button", { name: "Toggle theme" }));
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("save_preferences", { theme: "dark" }));
  expect(document.documentElement.classList.contains("dark")).toBe(true);
});
test("preview removes navigation, scripts and embedded browsing contexts", () => {
  const html = previewDocument(
    '<script>window.__TAURI_INTERNALS__.invoke("quit")</script><iframe src="https://example.com"></iframe><a href="https://example.com" target="_top">Leave</a><form action="https://example.com"><input></form><img src=x onerror="alert(1)"><meta http-equiv=refresh content="0;url=https://example.com">',
  );
  expect(html).not.toContain("<script>");
  expect(html).not.toContain("<iframe");
  expect(html).not.toContain("https://example.com");
  expect(html).not.toContain("onerror");
  expect(html).toContain("connect-src 'none'");
  expect(html).toContain("script-src 'none'");
});
test("long approvals require paging through the full request before enabling Allow", async () => {
  const binding = {
    id: "request",
    operation: "operation",
    argument_sha256: "full-args",
    scope: { workspace: "ws", session: "s", instance: "i", epoch: "1" },
    revision: "1",
    expires_ms: "9999999999999",
  };
  const snapshot = {
    version: { major: 1, minor: 0, schema: "fixture" },
    scope: binding.scope,
    revision: "1",
    watermark: "1",
    workspace_path: "/fixture",
    status: "awaiting_input",
    active_operation: "operation",
    messages: [],
    message_count: 0,
    pending: [
      {
        binding,
        title: "Shortened request",
        kind: "approval",
        options: [],
        evidence: "full-request",
        truncated: true,
      },
    ],
    model: "fixture",
    diagnostic: null,
    session_file: null,
    fixture_provider: true,
  };
  invoke.mockImplementation(async (name: string, args: any) => {
    if (name === "catalog") return { projects: [], theme: "light" };
    if (name === "bridge_view") return { snapshot, error: null, progress: "" };
    if (name === "bridge_command")
      return {
        kind: "reply",
        id: args.envelope.id,
        scope: snapshot.scope,
        revision: "1",
        durable: false,
        status: "page",
        data: {
          base64: btoa(args.envelope.command.offset === "0" ? "First page" : "Final page"),
          next: args.envelope.command.offset === "0" ? "10" : null,
        },
      };
  });
  render(<App />);
  const allow = await screen.findByRole("button", { name: "Allow this operation" });
  expect(allow.hasAttribute("disabled")).toBe(true);
  fireEvent.click(screen.getByRole("button", { name: "Read complete request" }));
  await screen.findByText("First page");
  expect(allow.hasAttribute("disabled")).toBe(true);
  fireEvent.click(screen.getByRole("button", { name: "Next request page" }));
  await screen.findByText("Final page");
  await waitFor(() => expect(allow.hasAttribute("disabled")).toBe(false));
  expect(
    invoke.mock.calls.filter(
      ([name, args]) => name === "bridge_command" && args.envelope.command.kind === "approve",
    ),
  ).toHaveLength(0);
});
