import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import { join, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { createInterface } from "node:readline";
import { randomUUID } from "node:crypto";
import { Ledger } from "../ledger.ts";
const root = resolve(import.meta.dirname, "../../../..");
const runtime = join(root, "reference/ag1-bridge");
const imp = async (p: string): Promise<any> =>
  import(pathToFileURL(join(runtime, "packages", p)).href);
const schema = JSON.parse(
  fs.readFileSync(join(root, "fixtures/protocol/roundtrip.json"), "utf8"),
)[0].data.version.schema;
const node = join(root, ".local/toolchains/node-v24.14.0-linux-x64/bin/node");
const wait = async (predicate: () => boolean, timeout = 15000) => {
  const until = Date.now() + timeout;
  while (!predicate()) {
    if (Date.now() > until) throw new Error("Timed out waiting for worker state");
    await new Promise((r) => setTimeout(r, 10));
  }
};
class Client {
  child: ChildProcessWithoutNullStreams;
  latest: any;
  frames: any[] = [];
  stderr = "";
  constructor(workspace: string, state: string) {
    this.child = spawn(
      node,
      [
        join(root, "product/compat/agent-worker/worker.ts"),
        runtime,
        state,
        workspace,
        schema,
        join(root, "product/target/debug/ag1-desktop"),
        join(root, ".local/bridge-runtime.json"),
        "true",
      ],
      { env: { PATH: "/usr/bin:/bin", HOME: join(state, "home"), AG1_LLAMA_CPP: "0" } },
    );
    this.child.stderr.on("data", (b) => (this.stderr += b));
    createInterface({ input: this.child.stdout }).on("line", (line) => {
      const f = JSON.parse(line).data;
      this.frames.push(f);
      if (f.kind === "snapshot") this.latest = f.snapshot;
    });
  }
  async ready() {
    try {
      await wait(() => !!this.latest);
    } catch (e) {
      throw new Error(String(e) + "\n" + this.stderr);
    }
  }
  async command(command: any, id = randomUUID(), overrides: any = {}) {
    const envelope = {
      version: this.latest.version,
      id,
      scope: this.latest.scope,
      expected_revision: this.latest.revision,
      command,
      ...overrides,
    };
    const before = this.frames.length;
    this.child.stdin.write(JSON.stringify({ type: "command", data: envelope }) + "\n");
    await wait(() => this.frames.slice(before).some((f) => f.kind === "reply" && f.id === id));
    return this.frames.slice(before).find((f) => f.kind === "reply" && f.id === id);
  }
  async idle() {
    await wait(() => this.latest.status === "idle");
  }
  async close() {
    this.child.stdin.end();
    await wait(() => this.child.exitCode !== null || this.child.signalCode !== null, 10000);
    assert.equal(this.child.exitCode, 0, this.stderr);
  }
}
function fixture() {
  const scratch = fs.mkdtempSync(join(os.tmpdir(), "ag1-p04-"));
  const workspace = join(scratch, "workspace"),
    state = join(scratch, "state");
  fs.mkdirSync(workspace);
  fs.mkdirSync(state);
  fs.writeFileSync(join(workspace, "README.md"), "# Real SDK fixture\n");
  return { scratch, workspace, state };
}
test("real SDK prompt → read tool → continuation, durable ID, renderer reconnect and restart", async () => {
  const f = fixture();
  let c = new Client(f.workspace, f.state);
  try {
    await c.ready();
    const id = randomUUID();
    const request = { kind: "prompt", input: { kind: "inline", text: "[tool] Read README.md" } };
    const accepted = await c.command(request, id);
    assert.equal(accepted.status, "accepted");
    assert(accepted.durable);
    await wait(() => c.latest.status === "idle" && c.latest.message_count >= 4);
    assert(
      c.latest.messages.some(
        (m: any) => m.role === "toolResult" && m.text.includes("Real SDK fixture"),
      ),
    );
    assert(c.latest.messages.some((m: any) => m.text.includes("Tool completed")));
    const count = c.latest.message_count;
    const duplicate = await c.command(request, id);
    assert.equal(duplicate.status, "completed");
    assert.equal(c.latest.message_count, count);
    const oldScope = c.latest.scope;
    await c.close();
    c = new Client(f.workspace, f.state);
    await c.ready();
    assert.equal(c.latest.message_count, count);
    assert.notEqual(c.latest.scope.instance, oldScope.instance);
    const stale = await c.command(request, randomUUID(), { scope: oldScope });
    assert.equal(stale.status, "rejected");
    const result = await c.command({ kind: "command_result", command: id });
    assert.equal(result.data.status, "completed");
    const changed = await c.command(
      { kind: "prompt", input: { kind: "inline", text: "different" } },
      id,
    );
    assert.equal(changed.status, "rejected");
  } finally {
    if (c.child.exitCode === null) await c.close();
    fs.rmSync(f.scratch, { recursive: true, force: true });
  }
});
test("routine mutations run without approval; user input, cancel and queued steering still work", async () => {
  const f = fixture();
  const c = new Client(f.workspace, f.state);
  try {
    await c.ready();
    await c.command({ kind: "prompt", input: { kind: "inline", text: "[approval] write a file" } });
    await c.idle();
    assert.equal(c.latest.pending.length, 0);
    assert.equal(fs.readFileSync(join(f.workspace, "approved.txt"), "utf8"), "Approved fixture mutation\n");
    await c.command({
      kind: "prompt",
      input: { kind: "inline", text: "[question] Ask a question" },
    });
    await wait(() => c.latest.pending.length === 1);
    const q = c.latest.pending[0];
    assert.equal(q.kind, "input");
    await c.command({
      kind: "session",
      action: "answer",
      args: { binding: q.binding, answer: "Use the local path" },
    });
    await c.idle();
    assert(c.latest.messages.some((m: any) => m.text.includes("Use the local path")));
    await c.command({ kind: "prompt", input: { kind: "inline", text: "[slow] think" } });
    await wait(() => c.latest.status === "running");
    await c.command({ kind: "session", action: "steer", args: { text: "Keep this direction" } });
    const stop = await c.command({ kind: "cancel", operation: c.latest.active_operation });
    assert.equal(stop.status, "completed");
    await c.idle();
  } finally {
    await c.close();
    fs.rmSync(f.scratch, { recursive: true, force: true });
  }
});
test("worker death records an interrupted turn and never replays its prompt", async () => {
  const f = fixture();
  let c = new Client(f.workspace, f.state);
  try {
    await c.ready();
    const id = randomUUID();
    await c.command({ kind: "prompt", input: { kind: "inline", text: "[slow] mutation" } }, id);
    await wait(() => c.latest.status === "running");
    c.child.kill("SIGKILL");
    await wait(() => c.child.signalCode !== null);
    c = new Client(f.workspace, f.state);
    await c.ready();
    assert.equal(c.latest.pending.length, 0);
    assert(c.latest.diagnostic.includes("interrupted"));
    const result = await c.command({ kind: "command_result", command: id });
    assert.equal(result.data.status, "interrupted");
  } finally {
    if (c.child.exitCode === null && c.child.signalCode === null) await c.close();
    fs.rmSync(f.scratch, { recursive: true, force: true });
  }
});
test("persistence failure cannot acknowledge or execute an intent", async () => {
  const f = fixture();
  const c = new Client(f.workspace, f.state);
  try {
    await c.ready();
    fs.renameSync(join(f.state, "commands"), join(f.state, "commands-retained"));
    fs.writeFileSync(join(f.state, "commands"), "unwritable directory fixture");
    const result = await c.command({
      kind: "prompt",
      input: { kind: "inline", text: "must not execute" },
    });
    assert.equal(result.status, "rejected");
    assert.equal(result.durable, false);
    assert.equal(c.latest.message_count, 0);
    fs.unlinkSync(join(f.state, "commands"));
    fs.renameSync(join(f.state, "commands-retained"), join(f.state, "commands"));
  } finally {
    await c.close();
    fs.rmSync(f.scratch, { recursive: true, force: true });
  }
});
test("B01 aggregate UTF-8 text cap preserves full mixed evidence before projection", async () => {
  const { capBridgeToolResult } = await imp("agent/dist/bridge-evidence.js");
  const f = fixture();
  process.env.AG1_BRIDGE_EVIDENCE = join(f.state, "overflow");
  try {
    const original = {
      content: [
        { type: "text", text: "é😀".repeat(50) },
        { type: "image", mimeType: "image/png", data: "aGVsbG8=" },
        { type: "text", text: "second".repeat(80) },
      ],
      details: { raw: true },
    };
    const result = capBridgeToolResult(original, 30);
    assert(
      result.content
        .filter((b: any) => b.type === "text")
        .reduce((n: number, b: any) => n + Buffer.byteLength(b.text), 0) <= 120,
    );
    assert.equal(result.content[1].data, original.content[1].data);
    assert.deepEqual(
      JSON.parse(
        fs.readFileSync(join(f.state, "overflow", result.details.bridgeEvidence + ".json"), "utf8"),
      ),
      original,
    );
    assert(!result.content.some((b: any) => b.text?.includes("�")));
    assert.equal(original.content[0].text, "é😀".repeat(50));
  } finally {
    delete process.env.AG1_BRIDGE_EVIDENCE;
    fs.rmSync(f.scratch, { recursive: true, force: true });
  }
});
test("B02 malformed/empty/invalid memory is retained and cannot be overwritten", async () => {
  const memory = await imp("coding-agent/dist/core/memory-store.js");
  const f = fixture();
  delete process.env.AG1_BRIDGE_PROJECT_STATE;
  fs.mkdirSync(join(f.workspace, ".ag1"));
  try {
    for (const bytes of [
      "{truncated",
      "",
      JSON.stringify({ version: 3, entries: [{ invalid: true }] }),
    ]) {
      fs.writeFileSync(memory.memoryStorePath(f.workspace), bytes);
      assert.throws(() => memory.loadMemoryStore(f.workspace), /Corrupt memory/);
      assert.throws(
        () => memory.saveMemoryStore(f.workspace, memory.emptyMemoryStore()),
        /Corrupt memory/,
      );
      assert.equal(fs.readFileSync(memory.memoryStorePath(f.workspace), "utf8"), bytes);
    }
    fs.unlinkSync(memory.memoryStorePath(f.workspace));
    memory.saveMemoryStore(f.workspace, memory.emptyMemoryStore());
    assert.equal(memory.loadMemoryStore(f.workspace).entries.length, 0);
  } finally {
    fs.rmSync(f.scratch, { recursive: true, force: true });
  }
});
test("B03 source snapshot hashes exactly the displayed bytes, including BOM/CRLF", async () => {
  const { readSourceSnapshot } = await imp("coding-agent/dist/core/source-revision.js");
  const { createHash } = await import("node:crypto");
  const f = fixture();
  try {
    const bytes = Buffer.from("\ufefffirst\r\né😀\r\n");
    fs.writeFileSync(join(f.workspace, "source.txt"), bytes);
    const value = readSourceSnapshot(f.workspace, "source.txt");
    assert.equal(value.text, bytes.toString());
    assert.equal(value.revision.sha256, createHash("sha256").update(bytes).digest("hex"));
    assert.throws(() => readSourceSnapshot(f.workspace, "source.txt", 1), /limit/);
  } finally {
    fs.rmSync(f.scratch, { recursive: true, force: true });
  }
});
test("B04 fork retains artifact closure and authenticated old cursors after parent removal", async () => {
  const { SessionManager } = await imp("coding-agent/dist/core/session-manager.js");
  const { ArtifactStore } = await imp("coding-agent/dist/core/artifact-store.js");
  const f = fixture();
  const sessions = join(f.state, "sessions");
  try {
    const manager = SessionManager.create(f.workspace, sessions);
    const parent = manager.getSessionId();
    const store = new ArtifactStore(join(sessions, "artifacts", parent), parent, {
      features: { typedEvidenceCursors: true },
    });
    const a = store.putText("Required historical evidence", {
      producer: { type: "system", name: "test" },
    });
    const other = store.putText("Unreferenced evidence", {
      producer: { type: "system", name: "test" },
    });
    const cursor = store.createReadCursor(a.id, 0, 100, "page");
    const unrelatedCursor = store.createReadCursor(other.id, 0, 100, "page");
    const leaf = manager.appendCustomEntry("evidence", { artifactId: a.id, cursor });
    const fork = manager.createBranchedSession(leaf);
    fs.rmSync(join(sessions, "artifacts", parent), { recursive: true });
    const restored = SessionManager.open(fork, sessions);
    const child = new ArtifactStore(
      join(sessions, "artifacts", restored.getSessionId()),
      restored.getSessionId(),
      { features: { typedEvidenceCursors: true } },
    );
    assert.equal(child.read(a.id).text, "Required historical evidence");
    assert.equal(child.decodeReadCursor(cursor).id, a.id);
    assert.throws(() => child.decodeReadCursor(unrelatedCursor));
    assert.throws(() => child.read(other.id));
    const stranger = new ArtifactStore(join(f.state, "stranger"), "stranger", {
      features: { typedEvidenceCursors: true },
    });
    assert.throws(() => stranger.decodeReadCursor(cursor));
  } finally {
    fs.rmSync(f.scratch, { recursive: true, force: true });
  }
});
test("Settings status/list and native file reads do not invalidate a mutation revision", async () => {
  const f = fixture();
  const c = new Client(f.workspace, f.state);
  try {
    await c.ready();
    const revision = c.latest.revision;
    for (const command of [
      { kind: "model", action: "status", args: {} },
      { kind: "session", action: "list", args: {} },
      { kind: "files", action: "read", args: { path: "README.md" } },
    ]) {
      const result = await c.command(command);
      assert.equal(result.status, "query");
      assert.equal(result.durable, false);
      assert.equal(c.latest.revision, revision);
    }
    const selected = await c.command({
      kind: "model",
      action: "select",
      args: { id: "Qwen3.8-27B-UD-Q4_K_S" },
    });
    assert.equal(selected.status, "completed");
    assert.equal(c.latest.model, "Qwen3.8-27B-UD-Q4_K_S");
  } finally {
    await c.close();
    fs.rmSync(f.scratch, { recursive: true, force: true });
  }
});
test("only an out-of-project write remains approval-gated", async () => {
  const f = fixture();
  const c = new Client(f.workspace, f.state);
  try {
    await c.ready();
    await c.command({
      kind: "prompt",
      input: { kind: "inline", text: "[outside-workspace] require a boundary review" },
    });
    await wait(() => c.latest.pending.length === 1);
    const binding = c.latest.pending[0].binding;
    assert.equal(c.latest.pending[0].kind, "approval");
    assert(!fs.existsSync(join(f.scratch, "outside-workspace.txt")));
    const result = await c.command({ kind: "approve", binding, allow: false });
    assert.equal(result.status, "completed");
    await c.idle();
    assert.equal(c.latest.pending.length, 0);
    assert(!fs.existsSync(join(f.scratch, "outside-workspace.txt")));
    assert.equal((await c.command({ kind: "approve", binding, allow: true })).status, "rejected");
  } finally {
    await c.close();
    fs.rmSync(f.scratch, { recursive: true, force: true });
  }
});
test("session replacement rejects active work; fork and resume preserve the committed branch", async () => {
  const f = fixture();
  const c = new Client(f.workspace, f.state);
  try {
    await c.ready();
    await c.command({
      kind: "prompt",
      input: { kind: "inline", text: "[slow] original conversation" },
    });
    await wait(() => c.latest.status === "running");
    for (const action of ["new", "fork", "resume"])
      assert.equal(
        (await c.command({ kind: "session", action, args: { file: "outside" } })).status,
        "rejected",
      );
    await c.idle();
    const parent = c.latest.session_file,
      parentId = c.latest.scope.session,
      count = c.latest.message_count;
    assert(count >= 2);
    assert.equal(
      (await c.command({ kind: "session", action: "fork", args: {} })).status,
      "completed",
    );
    assert.notEqual(c.latest.scope.session, parentId);
    assert.equal(c.latest.message_count, count);
    assert.equal(
      (await c.command({ kind: "session", action: "new", args: {} })).status,
      "completed",
    );
    assert.equal(c.latest.message_count, 0);
    assert.equal(
      (await c.command({ kind: "session", action: "resume", args: { file: parent } })).status,
      "completed",
    );
    assert.equal(c.latest.scope.session, parentId);
    assert.equal(c.latest.message_count, count);
    assert.equal(
      (
        await c.command({
          kind: "session",
          action: "resume",
          args: { file: join(f.workspace, "README.md") },
        })
      ).status,
      "rejected",
    );
    assert.equal(c.latest.scope.session, parentId);
  } finally {
    await c.close();
    fs.rmSync(f.scratch, { recursive: true, force: true });
  }
});
test("streaming completes with a paused transport, bounded projections and paged full evidence", async () => {
  const f = fixture();
  const c = new Client(f.workspace, f.state);
  try {
    await c.ready();
    const accepted = await c.command({
      kind: "prompt",
      input: { kind: "inline", text: "[stream] Exercise paused renderer transport" },
    });
    assert.equal(accepted.status, "accepted");
    c.child.stdout.pause();
    await new Promise((r) => setTimeout(r, 1000));
    c.child.stdout.resume();
    await c.idle();
    const progress = c.frames.filter((frame) => frame.kind === "progress");
    assert(progress.length > 0);
    assert(progress.length < 100, "Progress is coalesced, not one frame per delta");
    assert(progress.every((frame) => frame.text.length <= 4000));
    assert(!c.frames.some((frame) => frame.kind === "fatal"));
    const message = c.latest.messages.findLast((m: any) => m.role === "assistant");
    assert(message.text.length <= 1400);
    let offset = "0",
      full = "";
    for (let i = 0; i < 10; i++) {
      const page = await c.command({
        kind: "artifact_read",
        artifact: message.evidence,
        offset,
        limit: 32768,
      });
      assert.equal(page.status, "page");
      const bytes = Buffer.from(page.data.base64, "base64");
      assert(bytes.length <= 32768);
      full += bytes.toString();
      if (page.data.next === null) break;
      offset = page.data.next;
    }
    assert(JSON.parse(full).content[0].text.includes("Fixture event 1999:"));
  } finally {
    await c.close();
    fs.rmSync(f.scratch, { recursive: true, force: true });
  }
});
test("large routine writes complete without a review gate", async () => {
  const f = fixture();
  const c = new Client(f.workspace, f.state);
  try {
    await c.ready();
    await c.command({
      kind: "prompt",
      input: { kind: "inline", text: "[large-approval] Review all arguments" },
    });
    await c.idle();
    assert.equal(c.latest.pending.length, 0);
    assert.equal(
      fs.readFileSync(join(f.workspace, "approved.txt"), "utf8"),
      "Exact approved fixture.\n".repeat(3000),
    );
  } finally {
    await c.close();
    fs.rmSync(f.scratch, { recursive: true, force: true });
  }
});
test("Unicode and JSON escaping cannot overflow the snapshot projection", async () => {
  const f = fixture();
  const c = new Client(f.workspace, f.state);
  try {
    await c.ready();
    for (let i = 0; i < 12; i++) {
      const result = await c.command({
        kind: "prompt",
        input: { kind: "inline", text: `message ${i} ` + "😀\\\t".repeat(600) },
      });
      assert.equal(result.status, "accepted");
      await c.idle();
    }
    assert(!c.frames.some((frame) => frame.kind === "fatal"));
    assert(
      c.frames.every(
        (frame) => Buffer.byteLength(JSON.stringify({ type: "bridge", data: frame })) < 65536,
      ),
    );
    assert.equal(c.latest.message_count, 24);
  } finally {
    await c.close();
    fs.rmSync(f.scratch, { recursive: true, force: true });
  }
});
test("fatal protocol failure rejects already queued mutations without executing them", async () => {
  const f = fixture();
  const c = new Client(f.workspace, f.state);
  try {
    await c.ready();
    const id = randomUUID(),
      envelope = {
        version: c.latest.version,
        id,
        scope: c.latest.scope,
        expected_revision: c.latest.revision,
        command: {
          kind: "prompt",
          input: { kind: "inline", text: "must not execute after fatal" },
        },
      };
    c.child.stdin.write(
      JSON.stringify({ type: "bad" }) +
        "\n" +
        JSON.stringify({ type: "command", data: envelope }) +
        "\n",
    );
    await wait(() => c.frames.some((frame) => frame.kind === "reply" && frame.id === id));
    assert(c.frames.some((frame) => frame.kind === "fatal"));
    assert.equal(
      c.frames.find((frame) => frame.kind === "reply" && frame.id === id).status,
      "rejected",
    );
    assert.equal(c.latest.message_count, 0);
    assert(!fs.existsSync(join(f.state, "commands", id + ".command.json")));
  } finally {
    c.child.stdin.end();
    await wait(() => c.child.exitCode !== null);
    assert.equal(c.child.exitCode, 1);
    fs.rmSync(f.scratch, { recursive: true, force: true });
  }
});
test("B03 a pathname replacement during the descriptor read cannot publish mixed provenance", async () => {
  const { syncBuiltinESMExports } = await import("node:module");
  const { readSourceSnapshot } = await imp("coding-agent/dist/core/source-revision.js");
  const f = fixture();
  const file = join(f.workspace, "raced.txt");
  fs.writeFileSync(file, "original bytes");
  const read = fs.readSync;
  let changed = false;
  try {
    (fs as any).readSync = (...args: any[]) => {
      const count = (read as any)(...args);
      if (!changed) {
        changed = true;
        fs.renameSync(file, file + ".retained");
        fs.writeFileSync(file, "replacement bytes");
      }
      return count;
    };
    syncBuiltinESMExports();
    assert.throws(() => readSourceSnapshot(f.workspace, "raced.txt"), /changed during read/);
    assert(changed);
    assert.equal(fs.readFileSync(file + ".retained", "utf8"), "original bytes");
    assert.equal(fs.readFileSync(file, "utf8"), "replacement bytes");
  } finally {
    fs.readSync = read;
    syncBuiltinESMExports();
    fs.rmSync(f.scratch, { recursive: true, force: true });
  }
});
