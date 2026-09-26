import { expect, test, vi } from "vitest";
import { api, createCommandQueue, errorMessage, type View } from "./bridge";
import type { BridgeSnapshot, Command } from "./generated/protocol";
test("folder changes cannot follow the command queue into another chat", async () => {
  const scope = { workspace: "w", session: "a", instance: "i", epoch: "1" };
  const command = vi.fn();
  const queue = createCommandQueue({ view: vi.fn().mockResolvedValue({ snapshot: { scope: { ...scope, session: "b" } } }), command });
  await expect(queue({ kind: "runtime", action: "folder_access", args: { action: "add", path: "/project", revision: 1 } }, () => {}, scope)).rejects.toThrow("conversation changed");
  expect(command).not.toHaveBeenCalled();
});
const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

test.each(["rejected", "failed"])("%s native file-save conflicts explain safe recovery", async status => {
  invoke.mockReset();
  invoke.mockResolvedValue({ kind: "reply", status, data: { error: "Conflict" } });
  const snapshot = { version: { major: 1, minor: 0, schema: "test" }, scope: { workspace: "w", session: "s", instance: "i", epoch: "1" }, revision: "7" } as BridgeSnapshot;
  const command: Command = { kind: "files", action: "save", args: { path: "notes.txt", text: "retained edit", revision: "file-1" } };
  const id = "00000000-0000-4000-8000-000000000001";
  const error = await api.command(snapshot, command, id).catch(error => error);
  expect(error).toBeInstanceOf(Error);
  expect(error.message).toContain("Save conflict: the file changed on disk.");
  expect(error.message).toContain("Your unsaved text is still in the editor.");
  expect(error.message).toContain("Copy it somewhere safe before choosing Discard.");
  expect(error.message).toContain("Reopen the file from the file list");
  expect(error.message).toContain(`(command ${id})`);
  expect(error.message).not.toContain('{"error":');
  expect(invoke).toHaveBeenCalledTimes(1);
  expect(invoke).toHaveBeenCalledWith("bridge_command", { envelope: { version: snapshot.version, scope: snapshot.scope, expected_revision: "7", id, command } });
});

test.each([
  [{ kind: "files", action: "read", args: {} }, { error: "Conflict" }],
  [{ kind: "files", action: "save", args: {} }, { error: "PermissionDenied" }],
  [{ kind: "files", action: "save", args: {} }, { error: { code: "stale_revision" } }],
  [{ kind: "files", action: "save", args: {} }, { detail: "Conflict mentioned in unrelated detail" }],
  [{ kind: "model", action: "select", args: {} }, { error: "Conflict" }],
] as const)("unrelated failures retain their own diagnostics: %j", async (command, data) => {
  invoke.mockReset(); invoke.mockResolvedValue({ status: "rejected", data });
  const snapshot = { revision: "1" } as BridgeSnapshot;
  const id = "00000000-0000-4000-8000-000000000002";
  await expect(api.command(snapshot, command, id)).rejects.toThrow(`${"detail" in data ? data.detail : JSON.stringify(data)} (command ${id})`);
  expect(invoke).toHaveBeenCalledTimes(1);
});

test("a successful file response is not converted to an error", async () => {
  const reply = { status: "completed", data: { error: "Conflict" } };
  invoke.mockReset(); invoke.mockResolvedValue(reply);
  expect(await api.command({ revision: "1" } as BridgeSnapshot, { kind: "files", action: "save", args: {} })).toBe(reply);
});
test("back-to-back commands use acknowledged authority without waiting for a React render", async () => {
  let revision = 0n,
    inflight = 0,
    maxInflight = 0;
  const received: string[] = [];
  const snapshot = () =>
    ({
      version: { major: 1, minor: 0, schema: "test" },
      scope: { workspace: "ws", session: "session", instance: "worker", epoch: "1" },
      revision: String(revision),
    }) as BridgeSnapshot;
  const queue = createCommandQueue({
    view: async () => ({ snapshot: snapshot(), error: null, progress: "" }),
    command: async (s, _command, id = crypto.randomUUID()) => {
      inflight++;
      maxInflight = Math.max(maxInflight, inflight);
      expect(s.revision).toBe(String(revision));
      received.push(s.revision);
      await Promise.resolve();
      revision += 2n;
      inflight--;
      return {
        kind: "reply",
        id,
        scope: s.scope,
        revision: String(revision),
        durable: true,
        status: "completed",
        data: null,
      };
    },
  });
  const command: Command = { kind: "model", action: "select", args: { id: "fixture" } };
  await Promise.all(
    Array.from({ length: 20 }, () =>
      queue(command, (_view: View) => {
        /* Deliberately no render or ref update. */
      }),
    ),
  );
  expect(maxInflight).toBe(1);
  expect(received).toEqual(Array.from({ length: 20 }, (_, i) => String(i * 2)));
});
test("a rejected mutation is not replayed and later commands can proceed", async () => {
  let calls = 0;
  const queue = createCommandQueue({
    view: async () => ({
      snapshot: { revision: "1" } as BridgeSnapshot,
      error: null,
      progress: "",
    }),
    command: async () => {
      calls++;
      throw Error("Stale revision");
    },
  });
  const c: Command = { kind: "model", action: "select", args: {} };
  await expect(queue(c, () => {})).rejects.toThrow("Stale revision");
  expect(calls).toBe(1);
  await expect(queue(c, () => {})).rejects.toThrow();
  expect(calls).toBe(2);
  expect(errorMessage(new Error("Error: Error: Stale revision"))).toBe("Stale revision");
});

test("session navigation waits for its durable receipt before dispatching a prompt", async () => {
  let session = "yesterday", receipts = 0;
  const calls: string[] = [];
  const snapshot = () => ({ version: { major: 1, minor: 0, schema: "test" }, scope: { workspace: "ws", session, instance: "worker", epoch: "1" }, revision: "1" }) as BridgeSnapshot;
  const queue = createCommandQueue({
    view: async () => ({ snapshot: snapshot(), error: null, progress: "" }),
    command: async (s, c, id = crypto.randomUUID()) => {
      calls.push(c.kind);
      let data: any = { kind: "accepted", operation: "navigation" };
      if (c.kind === "command_result") {
        expect(s.scope.session).toBe("yesterday");
        if (++receipts === 2) { session = "fresh"; data = { kind: "settled", operation: { operation: "navigation", status: "completed" } }; }
      } else if (c.kind === "prompt") { expect(session).toBe("fresh"); expect(s.scope.session).toBe("fresh"); data = null; }
      return { kind: "reply", id, scope: s.scope, revision: "1", durable: true, status: "accepted", data };
    },
  });
  await Promise.all([queue({ kind: "session", action: "new", args: {} }, () => {}), queue({ kind: "prompt", input: { kind: "inline", text: "Fresh message" } }, () => {})]);
  expect(calls).toEqual(["session", "command_result", "command_result", "prompt"]);
});
test("failed session navigation rejects instead of pretending the new chat opened", async () => {
  const snapshot = { scope: { workspace: "ws", session: "old", instance: "i", epoch: "1" }, revision: "1" } as BridgeSnapshot;
  const queue = createCommandQueue({ view: async () => ({ snapshot, error: null, progress: "" }), command: async (_s, c, id = crypto.randomUUID()) => ({ kind: "reply", id, scope: snapshot.scope, revision: "1", durable: true, status: "accepted", data: c.kind === "session" ? { kind: "accepted", operation: "navigation" } : { kind: "settled", operation: { operation: "navigation", status: "failed" } } }) });
  await expect(queue({ kind: "session", action: "new", args: {} }, () => {})).rejects.toThrow("Chat navigation failed");
});
