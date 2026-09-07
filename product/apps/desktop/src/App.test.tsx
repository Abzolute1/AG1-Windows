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

test("late native theme bootstrap cannot overwrite a user's newer choice", async () => {
  let resolveCatalog!: (value: unknown) => void;
  invoke.mockImplementation((name: string) => name === "catalog" ? new Promise(resolve => { resolveCatalog = resolve; }) : Promise.resolve({ snapshot: null, error: null, progress: "" }));
  render(<App />);
  fireEvent.click(screen.getByRole("button", { name: "Toggle theme" }));
  const chosen = document.documentElement.classList.contains("dark");
  resolveCatalog({ projects: [], theme: chosen ? "light" : "dark" });
  await waitFor(() => expect(document.documentElement.classList.contains("dark")).toBe(chosen));
  await new Promise(resolve => setTimeout(resolve, 20));
  expect(document.documentElement.classList.contains("dark")).toBe(chosen);
});

vi.mock("./Editor", () => ({ default: ({ text, onChange }: { text: string; onChange: (text: string) => void }) => <textarea aria-label="File editor" value={text} onChange={e => onChange(e.target.value)} /> }));
function fileFixture(save: (args: any) => Promise<unknown>) {
  const snapshot = { version: { major: 1, minor: 0, schema: "fixture" }, scope: { workspace: "ws", session: "s", instance: "i", epoch: "1" }, revision: "1", watermark: "1", workspace_path: "/fixture", status: "idle", active_operation: null, messages: [], message_count: 0, pending: [], model: "fixture", diagnostic: null, session_file: null, fixture_provider: true };
  invoke.mockImplementation(async (name: string, args: any) => {
    if (name === "catalog") return { projects: [], theme: "light" };
    if (name === "bridge_view") return { snapshot, error: null, progress: "" };
    if (name === "bridge_command") {
      const command = args.envelope.command;
      const data = command.action === "list" ? ["notes.txt"] : command.action === "read" ? { path: "notes.txt", text: "original", revision: "file-1" } : await save(command.args);
      return { kind: "reply", id: args.envelope.id, scope: snapshot.scope, revision: "1", durable: false, status: "query", data };
    }
  });
}
async function editFile() {
  render(<App />);
  await waitFor(() => expect(invoke.mock.calls.some(([name]) => name === "bridge_view")).toBe(true));
  await new Promise(resolve => setTimeout(resolve, 30));
  fireEvent.click(screen.getByRole("button", { name: "Files" }));
  fireEvent.click(await screen.findByRole("button", { name: "notes.txt" }));
  const editor = await screen.findByRole("textbox", { name: "File editor" });
  fireEvent.change(editor, { target: { value: "local edit" } });
  return editor as HTMLTextAreaElement;
}
test("failed native save retains the dirty editor buffer", async () => {
  fileFixture(async () => { throw new Error("File revision conflict"); });
  const editor = await editFile();
  fireEvent.click(screen.getByRole("button", { name: "Save" }));
  await screen.findByText("File revision conflict");
  expect(editor.value).toBe("local edit");
  expect(screen.getByLabelText("Unsaved changes")).toBeTruthy();
});
test("a late successful save cannot discard typing entered after submission", async () => {
  let resolveSave!: (value: unknown) => void;
  fileFixture(() => new Promise(resolve => { resolveSave = resolve; }));
  const editor = await editFile();
  fireEvent.click(screen.getByRole("button", { name: "Save" }));
  await waitFor(() => expect(resolveSave).toBeTypeOf("function"));
  fireEvent.change(editor, { target: { value: "newer local edit" } });
  resolveSave({ path: "notes.txt", text: "local edit", revision: "file-2" });
  await waitFor(() => expect(screen.getByRole("button", { name: "Save" }).hasAttribute("disabled")).toBe(false));
  expect(editor.value).toBe("newer local edit");
  expect(screen.getByLabelText("Unsaved changes")).toBeTruthy();
});

test("native polling keeps one shortcut subscription and leaves the command queue usable", async () => {
  const snapshot = { version: { major: 1, minor: 0, schema: "fixture" }, scope: { workspace: "ws", session: "s", instance: "i", epoch: "1" }, revision: "1", watermark: "1", workspace_path: "/fixture", status: "idle", active_operation: null, messages: [], message_count: 0, pending: [], model: "fixture", diagnostic: null, session_file: null, fixture_provider: true };
  let polls = 0;
  invoke.mockImplementation(async (name: string, args: any) => {
    if (name === "catalog") return { projects: [], theme: "light" };
    if (name === "bridge_view") return { snapshot: { ...snapshot }, error: null, progress: String(++polls), details: { authority: "rust-isolated", messages: [], task: null, context: {}, memory: null, artifacts: [], activity: {}, models: [], capabilities: [] } };
    if (name === "bridge_command") return { kind: "reply", id: args.envelope.id, scope: snapshot.scope, revision: "1", durable: false, status: "rejected", data: { detail: "No extension host" } };
  });
  render(<App />);
  await waitFor(() => expect(polls).toBeGreaterThan(8));
  expect(invoke.mock.calls.filter(([name, args]) => name === "bridge_command" && args.envelope.command.action === "extension_settings")).toHaveLength(1);
});
