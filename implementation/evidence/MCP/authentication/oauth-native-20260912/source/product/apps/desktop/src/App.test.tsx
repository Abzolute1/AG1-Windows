import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { act, render, screen, fireEvent, cleanup, waitFor } from "@testing-library/react";
import App from "./App";
import { previewDocument } from "./preview";
vi.mock("@tauri-apps/api/event", () => ({ listen: async () => () => {} }));
const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
beforeEach(() => {
  localStorage.clear(); sessionStorage.clear();
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
  vi.stubGlobal("matchMedia", () => ({ matches: false }));
  invoke.mockImplementation(async (name: string) =>
    name === "catalog"
      ? { projects: [], theme: "light" }
      : { snapshot: null, error: null, progress: "" },
  );
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.clearAllMocks();
});
test("empty home bounds host polling and unmount cancels future requests", async () => {
  vi.useFakeTimers();
  const rendered = render(<App />);
  await act(async () => { await vi.advanceTimersByTimeAsync(2500); });
  const calls = () => invoke.mock.calls.filter(([name]) => name === "bridge_view").length;
  expect(calls()).toBeGreaterThan(0);
  expect(calls()).toBeLessThanOrEqual(3);
  const before = calls();
  rendered.unmount();
  await act(async () => { await vi.advanceTimersByTimeAsync(2500); });
  expect(calls()).toBe(before);
});
test("home opens a keyboard-accessible project dialog without starting a model", async () => {
  render(<App />);
  await screen.findByText("What’s on your mind?");
  fireEvent.keyDown(document, { key: "o", ctrlKey: true });
  expect(await screen.findByRole("dialog")).toBeTruthy();
  expect(screen.getByRole("textbox", { name: "Project folder" })).toBeTruthy();
  expect(invoke.mock.calls.some(([name]) => name === "bridge_command")).toBe(false);
  fireEvent.click(screen.getByText("Cancel"));
  await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
});
test("theme choice uses host preferences", async () => {
  render(<App />);
  await screen.findByText("What’s on your mind?");
  fireEvent.click(screen.getByRole("button", { name: "Settings" }));
  fireEvent.click(screen.getByRole("button", { name: "Choose theme" }));
  fireEvent.click(screen.getByRole("option", { name: /AG1 Graphite/ }));
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
  fireEvent.click(screen.getByRole("button", { name: "Settings" }));
  fireEvent.click(screen.getByRole("button", { name: "Choose theme" }));
  fireEvent.click(screen.getByRole("option", { name: /AG1 Graphite/ }));
  const chosen = document.documentElement.classList.contains("dark");
  resolveCatalog({ projects: [], theme: chosen ? "light" : "dark" });
  await waitFor(() => expect(document.documentElement.classList.contains("dark")).toBe(chosen));
  await new Promise(resolve => setTimeout(resolve, 20));
  expect(document.documentElement.classList.contains("dark")).toBe(chosen);
});

// Match Monaco's retained model: ordinary text props do not overwrite edits;
// only an explicit reset (or different file) replaces the model contents.
vi.mock("./Editor", async () => {
  const { useEffect, useState } = await import("react");
  return { default: function FixtureEditor({ text, path, resetKey, onChange }: { text: string; path: string; resetKey: string; onChange: (text: string) => void }) {
    const [model, setModel] = useState(text);
    useEffect(() => { setModel(text); }, [path, resetKey]);
    return <textarea aria-label="File editor" value={model} onChange={e => { setModel(e.target.value); onChange(e.target.value); }} />;
  } };
});
type FixtureFile = { path: string; text: string; revision: string };
function fileFixture(save: (args: any) => Promise<unknown>, read: () => FixtureFile | Promise<FixtureFile> = () => ({ path: "notes.txt", text: "original", revision: "file-1" })) {
  const snapshot = { version: { major: 1, minor: 0, schema: "fixture" }, scope: { workspace: "ws", session: "s", instance: "i", epoch: "1" }, revision: "1", watermark: "1", workspace_path: "/fixture", status: "idle", active_operation: null, messages: [], message_count: 0, pending: [], model: "fixture", diagnostic: null, session_file: null, fixture_provider: true };
  invoke.mockImplementation(async (name: string, args: any) => {
    if (name === "catalog") return { projects: [], theme: "light" };
    if (name === "bridge_view") return { snapshot, error: null, progress: "" };
    if (name === "bridge_command") {
      const command = args.envelope.command;
      const data = command.action === "list" ? ["notes.txt"] : command.action === "read" ? await read() : await save(command.args);
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
  // The real Monaco textbox appears only after editor initialization. This
  // retained-model mock renders before its passive reset effect has run.
  // Finish that initialization before simulating typing into a ready editor.
  await act(async () => {});
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
test("review opens beside the existing editor and closing preserves its dirty buffer", async () => {
  const save = vi.fn(async () => ({ view: "HEAD abc\n" }));
  fileFixture(save);
  const editor = await editFile();
  fireEvent.click(screen.getByRole("button", { name: "Changes" }));
  await screen.findByText("No tracked changes against HEAD.");
  expect(screen.getByRole("complementary", { name: "Review and artifacts" })).toBeTruthy();
  expect(editor.value).toBe("local edit");
  fireEvent.click(screen.getByRole("button", { name: "Close review panel" }));
  await waitFor(() => expect(screen.queryByRole("complementary", { name: "Review and artifacts" })).toBeNull());
  expect(editor.value).toBe("local edit"); expect(screen.getByLabelText("Unsaved changes")).toBeTruthy();
  expect(invoke.mock.calls.filter(([name, args]) => name === "bridge_command" && args.envelope.command.action === "save")).toHaveLength(0);
});
test("native save conflict explains recovery without changing or resubmitting the dirty editor", async () => {
  fileFixture(async () => null);
  const ordinary = invoke.getMockImplementation()!;
  invoke.mockImplementation(async (name: string, args: any) => {
    if (name === "bridge_command" && args.envelope.command.kind === "files" && args.envelope.command.action === "save")
      return { kind: "reply", status: "rejected", data: { error: "Conflict" } };
    return ordinary(name, args);
  });
  const editor = await editFile();
  fireEvent.click(screen.getByRole("button", { name: "Save" }));
  await screen.findByText(/Copy it somewhere safe before choosing Discard/);
  expect(editor.value).toBe("local edit");
  expect(screen.getByLabelText("Unsaved changes")).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Dismiss error" }));
  fireEvent.click(screen.getByRole("button", { name: "Refresh files" }));
  await waitFor(() => expect(screen.getByRole("button", { name: "Save" }).hasAttribute("disabled")).toBe(false));
  expect(editor.value).toBe("local edit");
  expect(screen.getByLabelText("Unsaved changes")).toBeTruthy();
  expect(invoke.mock.calls.filter(([name, args]) => name === "bridge_command" && args.envelope.command.kind === "files" && args.envelope.command.action === "save")).toHaveLength(1);
});
test("reopening the same file resets the retained editor model to the newly read bytes", async () => {
  let disk = { path: "notes.txt", text: "original", revision: "file-1" };
  fileFixture(async () => { throw new Error("File revision conflict"); }, () => disk);
  const editor = await editFile();
  disk = { ...disk, text: "externally changed", revision: "file-2" };
  fireEvent.click(screen.getByRole("button", { name: /^Save$/ }));
  await screen.findByText(/File revision conflict/);
  expect(editor.value).toBe("local edit");
  fireEvent.click(screen.getByRole("button", { name: "Discard" }));
  await waitFor(() => expect(editor.value).toBe("original"));
  fireEvent.click(screen.getByRole("button", { name: "notes.txt" }));
  await waitFor(() => expect(editor.value).toBe("externally changed"));
  expect(screen.queryByLabelText("Unsaved changes")).toBeNull();
});
test("a pending explicit reread cannot reset edits typed after it started", async () => {
  let defer = false, resolveRead: ((file: FixtureFile) => void) | undefined;
  fileFixture(async () => null, () => defer ? new Promise(resolve => { resolveRead = resolve; }) : { path: "notes.txt", text: "original", revision: "file-1" });
  const editor = await editFile();
  fireEvent.click(screen.getByRole("button", { name: "Discard" }));
  await waitFor(() => expect(editor.value).toBe("original"));
  defer = true;
  fireEvent.click(screen.getByRole("button", { name: "notes.txt" }));
  await waitFor(() => expect(resolveRead).toBeDefined());
  fireEvent.change(editor, { target: { value: "typed during reread" } });
  await act(async () => { resolveRead!({ path: "notes.txt", text: "external read", revision: "file-2" }); });
  expect(editor.value).toBe("typed during reread");
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
  await waitFor(() => expect(polls).toBeGreaterThan(2), { timeout: 5000 });
  expect(invoke.mock.calls.filter(([name, args]) => name === "bridge_command" && args.envelope.command.action === "extension_settings")).toHaveLength(1);
});

test("a fenced project shows recovery archives without dispatching or switching authority", async () => {
  invoke.mockImplementation(async (name: string) => {
    if (name === "catalog") return { projects: ["/fixture-migration"], theme: "light" };
    if (name === "open_project") throw new Error("Workspace is migrating; retry after recovery/resnapshot");
    if (name === "cutover_status") return { owner: "native", stage: "rollback_prepared", epoch: 3, state: "/private/native", transfer: { archive: "/private/original-archive", original_state: "/private/original" }, rollback: { archive: "/private/native-archive", destination: "/private/new-ts" } };
    return { snapshot: null, error: null, progress: "" };
  });
  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: "Open project" }));
  expect(await screen.findByRole("region", { name: "Workspace recovery" })).toBeTruthy();
  expect(screen.getByText("/private/native-archive")).toBeTruthy();
  expect(screen.getByText("/private/new-ts")).toBeTruthy();
  expect(screen.getByText("rollback_prepared")).toBeTruthy();
  expect(invoke.mock.calls.some(([name]) => name === "bridge_command")).toBe(false);
});

test("a model can be selected before a workspace and general chat sends through the native loop", async () => {
  let active = false;
  const snapshot = { version: { major: 1, minor: 0, schema: "fixture" }, scope: { workspace: "ws", session: "s", instance: "i", epoch: "1" }, revision: "1", watermark: "1", workspace_path: "/private/general-chat", status: "idle", active_operation: null, messages: [], message_count: 0, pending: [], model: "initial", diagnostic: null, session_file: null, fixture_provider: true };
  invoke.mockImplementation(async (name: string, args: any) => {
    if (name === "catalog") return { projects: [], chats: [], theme: "dark" };
    if (name === "desktop_models") return [{ id: "initial", provider: "fixture" }, { id: "second", provider: "fixture" }];
    if (name === "open_general") active = true;
    if (name === "bridge_command") { const c = args.envelope.command; if (c.kind === "model") snapshot.model = c.args.model_id; return { kind: "reply", status: "accepted", data: null }; }
    return { snapshot: active ? snapshot : null, error: null, progress: "" };
  });
  render(<App />); fireEvent.click(await screen.findByRole("button", { name: "Choose model" }));
  await waitFor(() => expect(document.querySelector(".selected-model-row")).toBeTruthy());
  fireEvent.click(document.querySelector(".selected-model-row")!);
  fireEvent.click(await screen.findByRole("button", { name: "second fixture" }));
  expect(invoke.mock.calls.some(([name]) => name === "open_general" || name === "open_project" || name === "bridge_command")).toBe(false);
  fireEvent.change(screen.getByRole("textbox", { name: "Message AG1" }), { target: { value: "Hello without a project" } });
  await waitFor(() => expect(screen.getByRole("button", { name: "Send message" }).hasAttribute("disabled")).toBe(false));
  fireEvent.click(screen.getByRole("button", { name: "Send message" }));
  await waitFor(() => expect(invoke.mock.calls.some(([name, a]) => name === "bridge_command" && a.envelope.command.kind === "prompt")).toBe(true));
  const commands = invoke.mock.calls.filter(([name]) => name === "bridge_command").map(([, a]) => a.envelope.command);
  expect(commands[0]).toEqual({ kind: "model", action: "select", args: { provider: "fixture", model_id: "second" } });
  expect(commands[1]).toEqual({ kind: "runtime", action: "approval_policy", args: { mode: "interactive" } });
  expect(commands[2]).toEqual({ kind: "prompt", input: { kind: "inline", text: "Hello without a project" } });
  expect(invoke.mock.calls.some(([name]) => name === "open_project")).toBe(false);
});
test("Tokyo and font size apply to the document through settings", async () => {
  render(<App />); fireEvent.click(screen.getByRole("button", { name: "Settings" }));
  fireEvent.click(screen.getByRole("button", { name: "Choose theme" }));
  fireEvent.click(screen.getByRole("option", { name: /Tokyo/ }));
  await waitFor(() => expect(document.documentElement.dataset.theme).toBe("tokyo"));
  expect(invoke).toHaveBeenCalledWith("save_preferences", { theme: "tokyo" });
  fireEvent.change(screen.getByRole("slider", { name: "Text size" }), { target: { value: "20" } });
  expect(document.documentElement.style.getPropertyValue("--chat-font-size")).toBe("20px");
});

test("models remain selectable during active work, and Settings exposes Stop without changing the running model", async () => {
  const snapshot = { version: { major: 1, minor: 0, schema: "fixture" }, scope: { workspace: "ws", session: "s", instance: "i", epoch: "1" }, revision: "1", watermark: "1", workspace_path: "/private/general-chat", status: "streaming", active_operation: "running-operation", messages: [], message_count: 0, pending: [], model: "initial", diagnostic: null, session_file: null, fixture_provider: true };
  invoke.mockImplementation(async (name: string, args: any) => {
    if (name === "catalog") return { projects: [], chats: [], theme: "dark" };
    if (name === "desktop_models") return [{ id: "initial", provider: "fixture" }, { id: "second", provider: "fixture" }];
    if (name === "bridge_command") return { kind: "reply", status: "accepted", data: null };
    return { snapshot, error: null, progress: "" };
  });
  render(<App />); await screen.findByRole("button", { name: "Stop generation" });
  fireEvent.click(screen.getByRole("button", { name: "Settings" }));
  fireEvent.click(screen.getByRole("button", { name: "Models" }));
  const second = await screen.findByRole("button", { name: "second" }); expect(second.hasAttribute("disabled")).toBe(false); fireEvent.click(second);
  expect(await screen.findByText("second will be used for your next message. The current response continues with initial.")).toBeTruthy();
  expect(invoke.mock.calls.some(([name]) => name === "bridge_command")).toBe(false);
  fireEvent.click(screen.getByRole("button", { name: "Stop current response" }));
  await waitFor(() => expect(invoke.mock.calls.some(([name, args]) => name === "bridge_command" && args.envelope.command.kind === "cancel" && args.envelope.command.operation === "running-operation")).toBe(true));
});
test("model choice and appearance remain usable while opening a chat is slow", async () => {
  let finish!: (value: unknown) => void;
  invoke.mockImplementation(async (name: string) => {
    if (name === "catalog") return { projects: [], chats: [], theme: "dark" };
    if (name === "desktop_models") return [{ id: "initial", provider: "fixture" }, { id: "second", provider: "fixture" }];
    if (name === "open_general") return await new Promise(resolve => { finish = resolve; });
    return { snapshot: null, error: null, progress: "" };
  });
  render(<App />); fireEvent.click(screen.getAllByRole("button", { name: "New chat" })[0]);
  await screen.findByText("Opening chat / preparing request…");
  fireEvent.click(screen.getByRole("button", { name: "Settings" }));
  fireEvent.click(screen.getByRole("button", { name: "Models" }));
  const second = await screen.findByRole("button", { name: "second" }); expect(second.hasAttribute("disabled")).toBe(false); fireEvent.click(second);
  fireEvent.click(screen.getByRole("button", { name: "Appearance" }));
  fireEvent.click(screen.getByRole("button", { name: "Choose theme" }));
  fireEvent.click(screen.getByRole("option", { name: /Tokyo/ })); expect(document.documentElement.dataset.theme).toBe("tokyo");
  finish({ snapshot: null, error: null, progress: "" });
  await waitFor(() => expect(screen.queryByText("Opening chat / preparing request…")).toBeNull());
});

test("Tool approvals opens policy choices without opening diagnostics or requiring a workspace", async () => {
  render(<App />); await screen.findByText("What’s on your mind?");
  fireEvent.click(screen.getByRole("button", { name: "Tool approvals" }));
  expect(screen.getAllByRole("radio")).toHaveLength(3);
  fireEvent.click(screen.getByRole("radio", { name: "Bypass approvals" }));
  expect(screen.queryByRole("radiogroup")).toBeNull(); expect(screen.queryByLabelText("Task and runtime details")).toBeNull();
  await screen.findByRole("dialog", { name: "Bypass tool approvals?" });
  expect(document.querySelector("button[aria-label='Tool approvals']")?.textContent).toContain("Ask before changes");
  fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
  await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  expect(document.querySelector("button[aria-label='Tool approvals']")?.textContent).toContain("Ask before changes");
  fireEvent.click(screen.getByRole("button", { name: "Settings" }));
  fireEvent.click(screen.getByRole("button", { name: "Permissions" }));
  fireEvent.click(screen.getByRole("radio", { name: "Bypass approvals" }));
  await screen.findByRole("dialog", { name: "Bypass tool approvals?" });
  fireEvent.keyDown(screen.getByRole("dialog"), { key: "Escape" });
  await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  expect(screen.getByRole("radio", { name: "Ask before changes" }).getAttribute("aria-checked")).toBe("true");
  fireEvent.click(screen.getByRole("radio", { name: "Bypass approvals" }));
  fireEvent.click(await screen.findByRole("button", { name: "Enable bypass" }));
  await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  expect(screen.getByRole("radio", { name: "Bypass approvals" }).getAttribute("aria-checked")).toBe("true");

  expect(invoke.mock.calls.some(([name]) => name === "bridge_command" || name === "open_general")).toBe(false);
});

test("typing while a new chat opens is preserved when opening finishes", async () => {
  let finish!: (value: unknown) => void;
  invoke.mockImplementation(async (name: string) => name === "catalog" ? { projects: [], chats: [], theme: "dark" } : name === "open_general" ? new Promise(resolve => { finish = resolve; }) : { snapshot: null, progress: "", error: null });
  render(<App />); fireEvent.click(screen.getAllByRole("button", { name: "New chat" })[0]);
  await waitFor(() => expect(finish).toBeTypeOf("function"));
  fireEvent.change(screen.getByRole("textbox", { name: "Message AG1" }), { target: { value: "Keep this newly typed message" } });
  finish({ snapshot: null, progress: "", error: null });
  await waitFor(() => expect(screen.queryByText("Opening chat / preparing request…")).toBeNull());
  expect((screen.getByRole("textbox", { name: "Message AG1" }) as HTMLTextAreaElement).value).toBe("Keep this newly typed message");
});

function modelFixture() {
  const state = { active: false, reject: false, unchanged: false, delay: null as Promise<void> | null };
  const snapshot = { version: { major: 1, minor: 0, schema: "fixture" }, scope: { workspace: "ws", session: "yesterday", instance: "i", epoch: "1" }, revision: "1", watermark: "1", workspace_path: "/private/general-chat", status: "idle", active_operation: null, messages: [], message_count: 8, pending: [], model: "second", diagnostic: null, session_file: null, fixture_provider: true };
  const commands: any[] = [];
  invoke.mockImplementation(async (name: string, args: any) => {
    if (name === "catalog") return { projects: [], chats: [], theme: "dark" };
    if (name === "desktop_models") return [{ id: "initial", provider: "fixture" }, { id: "second", provider: "fixture" }];
    if (name === "open_general") { if (state.delay) await state.delay; state.active = true; }
    if (name === "bridge_command") {
      const c = args.envelope.command; commands.push(c);
      if (c.kind === "session" && c.action === "new") { snapshot.scope = { ...snapshot.scope, session: "fresh" }; snapshot.message_count = 0; }
      if (c.kind === "model" && c.action === "select") {
        if (state.reject) return { kind: "reply", status: "rejected", data: { detail: "Model unavailable" } };
        if (!state.unchanged) snapshot.model = c.args.model_id;
      }
      return { kind: "reply", status: "accepted", data: null };
    }
    return { snapshot: state.active ? { ...snapshot, scope: { ...snapshot.scope } } : null, error: null, progress: "" };
  });
  return { state, snapshot, commands };
}
test("sending from New chat creates a fresh session and uses the model displayed before opening", async () => {
  const { commands } = modelFixture(); render(<App />);
  await waitFor(() => expect(screen.getByRole("button", { name: "Choose model" }).textContent).toContain("initial"));
  fireEvent.change(screen.getByRole("textbox", { name: "Message AG1" }), { target: { value: "Fresh conversation" } });
  fireEvent.click(screen.getByRole("button", { name: "Send message" }));
  await waitFor(() => expect(commands.some(c => c.kind === "prompt")).toBe(true));
  expect(commands.filter(c => ["session", "model", "prompt"].includes(c.kind))).toEqual([
    { kind: "session", action: "new", args: {} },
    { kind: "model", action: "select", args: { provider: "fixture", model_id: "initial" } },
    { kind: "prompt", input: { kind: "inline", text: "Fresh conversation" } },
  ]);
  const prompt = invoke.mock.calls.find(([n, a]) => n === "bridge_command" && a.envelope.command.kind === "prompt");
  expect(prompt![1].envelope.scope.session).toBe("fresh");
});
test("subscription model default effort is applied before the first prompt", async () => {
  const { commands } = modelFixture(); const base = invoke.getMockImplementation()!;
  invoke.mockImplementation(async (name: string, args: any) => {
    if (name === "desktop_models") return [{ id: "initial", provider: "fixture", reasoningControls: {kind:"levels",source:"provider",default:"high",modes:[{id:"low",label:"Low",level:"low"},{id:"high",label:"High",level:"high"},{id:"xhigh",label:"Extra high",level:"xhigh"}]} }];
    return base(name,args);
  });
  render(<App />); await waitFor(()=>expect(screen.getByRole("button",{name:"Choose model"}).textContent).toContain("initial"));
  fireEvent.change(screen.getByRole("textbox",{name:"Message AG1"}),{target:{value:"Use the model default"}});fireEvent.click(screen.getByRole("button",{name:"Send message"}));
  await waitFor(()=>expect(commands.some(c=>c.kind==="prompt")).toBe(true));
  const reasoning=commands.findIndex(c=>c.kind==="model"&&c.action==="reasoning");expect(commands[reasoning].args.mode).toBe("high");expect(reasoning).toBeLessThan(commands.findIndex(c=>c.kind==="prompt"));
});
test.each(["reject", "unchanged"] as const)("a %s model selection retains the draft and never sends to the previous model", async failure => {
  const { state, commands } = modelFixture(); state[failure] = true; render(<App />);
  await waitFor(() => expect(screen.getByRole("button", { name: "Choose model" }).textContent).toContain("initial"));
  fireEvent.change(screen.getByRole("textbox", { name: "Message AG1" }), { target: { value: "Do not fall back" } });
  fireEvent.click(screen.getByRole("button", { name: "Send message" }));
  await screen.findByRole("alert");
  expect(commands.some(c => c.kind === "prompt")).toBe(false);
  expect((screen.getByRole("textbox", { name: "Message AG1" }) as HTMLTextAreaElement).value).toBe("Do not fall back");
});
test("changing the picker while a submitted request opens preserves the submitted choice", async () => {
  const { state, commands } = modelFixture(); let finish!: () => void; state.delay = new Promise(resolve => { finish = resolve; }); render(<App />);
  await waitFor(() => expect(screen.getByRole("button", { name: "Choose model" }).textContent).toContain("initial"));
  fireEvent.change(screen.getByRole("textbox", { name: "Message AG1" }), { target: { value: "Use initial" } });
  fireEvent.click(screen.getByRole("button", { name: "Send message" }));
  await waitFor(() => expect(invoke.mock.calls.some(([n]) => n === "open_general")).toBe(true));
  fireEvent.click(screen.getByRole("button", { name: "Choose model" }));
  const switchModel = document.querySelector(".selected-model-row"); if (switchModel) fireEvent.click(switchModel);
  fireEvent.click(await screen.findByRole("button", { name: "second fixture" }));
  expect(await screen.findByText("This request uses initial. second is selected for your next message.")).toBeTruthy();
  await act(async () => finish());
  await waitFor(() => expect(commands.some(c => c.kind === "prompt")).toBe(true));
  expect(commands.find(c => c.kind === "model").args.model_id).toBe("initial");
});

test("a delayed view from yesterday cannot replace the new session after submission", async () => {
  const { commands, snapshot } = modelFixture();
  const base = invoke.getMockImplementation()!;
  const oldView = { snapshot: { ...snapshot, scope: { ...snapshot.scope } }, error: null, progress: "" };
  let first = true, finish!: (v: unknown) => void;
  invoke.mockImplementation(async (name: string, args: any) => {
    if (name === "catalog") return { projects: [], theme: "dark", chats: [{ session: "yesterday", workspace: snapshot.workspace_path, title: "Yesterday", general: true }, { session: "fresh", workspace: snapshot.workspace_path, title: "Fresh", general: true }] };
    if (name === "bridge_view" && first) { first = false; return new Promise(resolve => { finish = resolve; }); }
    return base(name, args);
  });
  render(<App />);
  await waitFor(() => expect(screen.getByRole("button", { name: "Choose model" }).textContent).toContain("initial"));
  fireEvent.change(screen.getByRole("textbox", { name: "Message AG1" }), { target: { value: "Fresh message" } });
  fireEvent.click(screen.getByRole("button", { name: "Send message" }));
  await waitFor(() => expect(commands.some(c => c.kind === "prompt")).toBe(true));
  await waitFor(() => expect(document.querySelector(".chat-heading")?.textContent).toBe("Fresh"));
  await act(async () => { finish(oldView); await new Promise(resolve => setTimeout(resolve, 50)); });
  expect(document.querySelector(".chat-heading")?.textContent).toBe("Fresh");
});

test("New chat stays in the attached project and does not reopen or unload its host", async () => {
  const { state, snapshot, commands } = modelFixture(); state.active = true; snapshot.workspace_path = "/project/already-open";
  render(<App />); await screen.findByRole("button", { name: "Task and activity" });
  fireEvent.click(screen.getAllByRole("button", { name: "New chat" })[0]);
  await waitFor(() => expect(commands.some(c => c.kind === "session" && c.action === "new")).toBe(true));
  expect(invoke.mock.calls.some(([name]) => name === "open_general" || name === "open_project")).toBe(false);
  expect(snapshot.workspace_path).toBe("/project/already-open");
  expect(commands.some(c => c.kind === "model")).toBe(false);
});

test("bypass consent does not survive restart or enter another chat, and safe policy reaches the backend", async () => {
  const values = new Map<string, string>([["ag1.approval", "disabled"]]);
  const storage = { getItem: (k: string) => values.get(k) ?? null, setItem: (k: string, v: string) => values.set(k, v), removeItem: (k: string) => values.delete(k) };
  const original = Object.getOwnPropertyDescriptor(window, "localStorage");
  Object.defineProperty(window, "localStorage", { configurable: true, value: storage });
  try {
    const { commands } = modelFixture();
    const app = render(<App />);
    await waitFor(() => expect(screen.getByRole("button", { name: "Choose model" }).textContent).toContain("initial"));
    expect(screen.getByRole("button", { name: "Tool approvals" }).textContent).toContain("Ask before changes");
    fireEvent.click(screen.getByRole("button", { name: "Tool approvals" }));
    fireEvent.click(screen.getByRole("radio", { name: "Bypass approvals" }));
    fireEvent.click(await screen.findByRole("button", { name: "Enable bypass" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    fireEvent.change(screen.getByRole("textbox", { name: "Message AG1" }), { target: { value: "Consented chat" } });
    fireEvent.click(screen.getByRole("button", { name: "Send message" }));
    await waitFor(() => expect(commands.some(c => c.kind === "prompt")).toBe(true));
    expect(commands.find(c => c.action === "approval_policy").args.mode).toBe("disabled");
    fireEvent.click(screen.getAllByRole("button", { name: "New chat" })[0]);
    await waitFor(() => expect(screen.getByRole("button", { name: "Tool approvals" }).textContent).toContain("Ask before changes"));
    commands.length = 0;
    fireEvent.change(screen.getByRole("textbox", { name: "Message AG1" }), { target: { value: "Fresh safe chat" } });
    fireEvent.click(screen.getByRole("button", { name: "Send message" }));
    await waitFor(() => expect(commands.some(c => c.kind === "prompt")).toBe(true));
    expect(commands.find(c => c.action === "approval_policy").args.mode).toBe("interactive");
    // Simulate an app restart with the old persisted bypass value still present.
    app.unmount(); render(<App />);
    expect(screen.getByRole("button", { name: "Tool approvals" }).textContent).toContain("Ask before changes");
  } finally {
    cleanup();
    if (original) Object.defineProperty(window, "localStorage", original);
  }
});


test("deleting three conversations creates no replacement sidebar chats, and old empty rows disappear", async () => {
  const { state, snapshot } = modelFixture(); state.active = true;
  const base = invoke.getMockImplementation()!;
  let chats: any[] = ["yesterday", "second-chat", "third-chat", "old-empty"].map(session => ({ session, workspace: snapshot.workspace_path, title: session === "old-empty" ? "New chat" : session, general: true }));
  const rows = () => chats.map(c => ({ ...c, empty: c.session === "old-empty" })).concat(snapshot.scope.session === "fresh" ? [{ session: "fresh", workspace: snapshot.workspace_path, title: "New chat", general: true, empty: true }] : []);
  invoke.mockImplementation(async (name: string, args: any) => {
    if (name === "catalog") return { projects: [], theme: "dark", chats: [...chats] };
    if (name === "save_chat") { const i = chats.findIndex(c => c.session === args.chat.session); if (i < 0) chats.push(args.chat); else chats[i] = args.chat; return; }
    if (name === "bridge_command" && args.envelope.command.action === "catalog") return { kind: "reply", status: "accepted", data: { chats: rows(), next: null } };
    if (name === "bridge_command" && args.envelope.command.action === "resume") { snapshot.scope = { ...snapshot.scope, session: args.envelope.command.args.session }; snapshot.message_count = 2; }
    if (name === "select_conversation") { snapshot.scope = { ...snapshot.scope, session: args.session ?? "fresh" }; snapshot.message_count = args.session ? 2 : 0; }
    const result = await base(name, args);
    if (result?.snapshot) result.details = { context: {}, messages: [] };
    return result;
  });
  render(<App />);
  await waitFor(() => expect(document.querySelectorAll(".chat-row")).toHaveLength(3));
  for (const title of ["yesterday", "second-chat", "third-chat"]) {
    fireEvent.click(document.querySelector(`.chat-row button[title="${title}"]`)!);
    await waitFor(() => expect(document.querySelector(".chat-row.selected")?.textContent).toContain(title));
    const row = screen.getByLabelText(`Options for ${title}`).closest(".chat-row")!;
    fireEvent.click(Array.from(row.querySelectorAll("button")).find(b => b.textContent === "Delete chat")!);
    fireEvent.click(await screen.findByRole("button", { name: "Move to Trash" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    await waitFor(() => expect(document.querySelector(`.chat-row button[title="${title}"]`)).toBeNull());
  }
  expect(document.querySelectorAll(".chat-row")).toHaveLength(0);
  expect(chats.some(c => c.session === "fresh")).toBe(false);
  expect(chats.filter(c => c.deleted)).toHaveLength(3);
  expect(screen.getByRole("textbox", { name: "Message AG1" })).toBeTruthy();
});

test("pasting an image creates a removable preview without sending or replacing ordinary text", async () => {
  URL.createObjectURL = vi.fn(() => "blob:pasted"); URL.revokeObjectURL = vi.fn();
  render(<App />);
  const composer = screen.getByRole("textbox", { name: "Message AG1" });
  fireEvent.change(composer, { target: { value: "Describe this" } });
  const file = new File(["png bytes"], "pasted.png", { type: "image/png" });
  fireEvent.paste(composer, { clipboardData: { files: [file], getData: () => "" } });
  expect(await screen.findByRole("button", { name: "Preview pasted.png" })).toBeTruthy();
  expect((composer as HTMLTextAreaElement).value).toBe("Describe this");
  expect(invoke.mock.calls.some(([name]) => name === "open_general")).toBe(false);
  fireEvent.click(screen.getByRole("button", { name: "Remove pasted.png" }));
  await waitFor(() => expect(screen.queryByRole("button", { name: "Preview pasted.png" })).toBeNull());
});

test("native navigation keeps background work alive, restores drafts, and Stop targets the visible conversation", async () => {
  const make = (session: string) => ({ snapshot: { version: { major: 1, minor: 0, schema: "fixture" }, scope: { workspace: "ws", session, instance: session, epoch: "1" }, revision: "1", watermark: "1", workspace_path: "/private/general-chat", status: "streaming", active_operation: `run-${session}`, messages: [], message_count: 1, pending: [], model: "initial", diagnostic: null, session_file: null, fixture_provider: true }, details: { context: { selected_model: { provider: "fixture" } }, activity: {}, messages: [{ id: `user-${session}`, role: "user", blocks: [{ kind: "text", text: `Question ${session}` }] }] }, error: null, progress: "" });
  const views = { a: make("a"), b: make("b") }; let selected: "a" | "b" = "a";
  const chats = ["a", "b"].map(session => ({ session, workspace: "/private/general-chat", title: `Chat ${session}`, general: true }));
  invoke.mockImplementation(async (name: string, args: any) => {
    if (name === "catalog") return { projects: [], chats, theme: "dark" };
    if (name === "desktop_models") return [{ id: "initial", provider: "fixture" }];
    if (name === "bridge_view") return views[selected];
    if (name === "select_conversation") { selected = args.session; return views[selected]; }
    if (name === "conversation_statuses") return chats.map(c => ({ ...c, status: "running", pending: 0, error: null }));
    if (name === "bridge_live") return null;
    if (name === "bridge_command") return { kind: "reply", status: "query", data: args.envelope.command.action === "catalog" ? { chats, next: null } : null };
    return null;
  });
  render(<App />); await screen.findByText("Question a");
  fireEvent.change(screen.getByRole("textbox", { name: "Message AG1" }), { target: { value: "A follow-up draft" } });
  fireEvent.click(screen.getByTitle("Chat b")); await screen.findByText("Question b");
  expect(screen.queryByText(/Stop the current response before switching/)).toBeNull();
  expect(invoke.mock.calls.some(([name, args]) => name === "bridge_command" && ["cancel", "session"].includes(args.envelope.command.kind) && args.envelope.command.action !== "catalog")).toBe(false);
  fireEvent.click(screen.getByRole("button", { name: "Stop generation" }));
  await waitFor(() => expect(invoke.mock.calls.some(([name, args]) => name === "bridge_command" && args.envelope.command.kind === "cancel" && args.envelope.scope.session === "b" && args.envelope.command.operation === "run-b")).toBe(true));
  fireEvent.click(screen.getByTitle("Chat a")); await screen.findByText("Question a");
  expect((screen.getByRole("textbox", { name: "Message AG1" }) as HTMLTextAreaElement).value).toBe("A follow-up draft");
});

test("project plus browses folders and cancellation preserves the selected workspace path", async () => {
  const base = invoke.getMockImplementation()!;
  let selection: string | null = "/home/alex/New project";
  invoke.mockImplementation(async (name: string, ...args: unknown[]) => name === "choose_chat_folder" ? selection : base(name, ...args));
  render(<App />);
  await screen.findByText("What’s on your mind?");
  fireEvent.click(screen.getByRole("button", { name: "Add project" }));
  await waitFor(() => expect((screen.getByRole("textbox", { name: "Project folder" }) as HTMLInputElement).value).toBe(selection));
  expect(invoke).toHaveBeenCalledWith("choose_chat_folder");
  expect(invoke.mock.calls.some(([name]) => name === "open_project")).toBe(false);
  selection = null;
  fireEvent.click(screen.getByRole("button", { name: "Browse folders…" }));
  await waitFor(() => expect(invoke.mock.calls.filter(([name]) => name === "choose_chat_folder")).toHaveLength(2));
  expect((screen.getByRole("textbox", { name: "Project folder" }) as HTMLInputElement).value).toBe("/home/alex/New project");
});
