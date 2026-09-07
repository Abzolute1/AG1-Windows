// One executing SDK session per supervised worker. Renderer data is never executable code.
import fs from "node:fs";
import { join, resolve, dirname } from "node:path";
import { pathToFileURL } from "node:url";
import { randomUUID } from "node:crypto";
import { spawnSync } from "node:child_process";
import { Ledger, atomic, digest, syncTree } from "./ledger.ts";
import { runtimeGuard } from "./runtime.ts";
import type {
  CommandEnvelope,
  BridgeFrame,
  BridgeSnapshot,
  BridgeQuestion,
  ApprovalBinding,
  Scope,
} from "../../apps/desktop/src/generated/protocol.ts";
const [runtime, directory, workspace, schema, nativeHost, runtimeConfig, fixture = "false"] =
  process.argv.slice(2);
if (!runtime || !directory || !workspace || !schema || !nativeHost || !runtimeConfig)
  throw new Error("Missing explicit worker launch context");
const root = fs.realpathSync(workspace);
const state = resolve(directory);
if (state === root || state.startsWith(root + "/"))
  throw new Error("Agent destination must be private and outside the workspace");
fs.mkdirSync(state, { recursive: true, mode: 0o700 });
process.umask(0o077);
process.env.AG1_CODING_AGENT_DIR = join(state, "agent");
process.env.AG1_BRIDGE_PROJECT_STATE = join(state, "project");
process.env.AG1_BRIDGE_EVIDENCE = join(state, "overflow");
for (const dir of ["agent", "project", "overflow", "sessions", "evidence"])
  fs.mkdirSync(join(state, dir), { recursive: true, mode: 0o700 });
const imp = async (path: string): Promise<any> =>
  import(pathToFileURL(join(runtime, "packages", path)).href);
// Libraries occasionally log diagnostics. Reserve stdout exclusively for bounded protocol frames.
console.log = (...args: unknown[]) => console.error(...args);
const { createAgentSession } = await imp("coding-agent/dist/core/sdk.js");
const { SessionManager } = await imp("coding-agent/dist/core/session-manager.js");
const { AuthStorage } = await imp("coding-agent/dist/core/auth-storage.js");
const { SettingsManager } = await imp("coding-agent/dist/core/settings-manager.js");
const { ModelRegistry } = await imp("coding-agent/dist/core/model-registry.js");
const { buildTurnSystemPromptAddendum } = await imp("coding-agent/dist/core/turn-addenda.js");
const { createExtensionRuntime } = await imp("coding-agent/dist/core/extensions/loader.js");
const { evaluateToolSafety } = await imp("coding-agent/dist/core/safety-policy.js");
const { AssistantMessageEventStream } = await imp("ai/dist/utils/event-stream.js");
const llama = await imp("coding-agent/dist/core/llamacpp-runtime.js");
const { installBridgeRuntimeGuard } = await imp("coding-agent/dist/core/bridge-runtime-guard.js");
const config = JSON.parse(fs.readFileSync(runtimeConfig, "utf8"));
const profilePath = join(state, "agent", "llamacpp-model-profiles.json");
if (!fs.existsSync(profilePath))
  fs.copyFileSync(config.profile_file, profilePath, fs.constants.COPYFILE_EXCL);
installBridgeRuntimeGuard(runtimeGuard(config, profilePath, llama.buildLlamaCppServerArgs));
const ledger = new Ledger(join(state, "commands"));
const interrupted = ledger.recover();
const version = { major: 1, minor: 0, schema };
let manager: any,
  session: any,
  active: string | null = null,
  running: Promise<void> | undefined,
  fatal = false;
let scope: Scope = {
  workspace: "ws-" + digest(root).slice(0, 24),
  session: "initial",
  instance: randomUUID(),
  epoch: "1",
};
let diagnostic: string | null = interrupted
  ? `${interrupted} interrupted command(s); inspect saved results before retrying.`
  : null;
let pending = new Map<
  string,
  { view: BridgeQuestion; resolve: (value: any) => void; timer: ReturnType<typeof setTimeout> }
>();
let queued = 0,
  serial: Promise<void> = Promise.resolve(),
  streamText = "",
  latestSnapshot: BridgeSnapshot | undefined;
let modelId = Object.keys(config.models)[0];
const activePath = join(state, "active.json");
if (fs.existsSync(activePath)) {
  const restored = JSON.parse(fs.readFileSync(activePath, "utf8"));
  modelId = restored.model ?? modelId;
}
let outputQueue: string[] = [],
  outputBytes = 0,
  blocked = false;
function flush() {
  while (!blocked && outputQueue.length) {
    const line = outputQueue.shift()!;
    outputBytes -= Buffer.byteLength(line);
    blocked = !process.stdout.write(line);
  }
}
process.stdout.on("drain", () => {
  blocked = false;
  flush();
});
function send(frame: BridgeFrame) {
  const line = JSON.stringify({ type: "bridge", data: frame }) + "\n";
  if (Buffer.byteLength(line) > 65536)
    throw new Error("Projection exceeded 64 KiB; use evidence paging");
  if (outputBytes + Buffer.byteLength(line) > 1024 * 1024)
    throw new Error("Renderer transport stalled; reconnect required");
  outputQueue.push(line);
  outputBytes += Buffer.byteLength(line);
  flush();
}
function evidence(value: unknown) {
  const id = digest(value),
    path = join(state, "evidence", id + ".json");
  if (!fs.existsSync(path)) atomic(path, value);
  return id;
}
function textOf(message: any): string {
  return typeof message.content === "string"
    ? message.content
    : (message.content ?? [])
        .map(
          (b: any) =>
            b.text ??
            (b.type === "toolCall"
              ? `${b.name} ${JSON.stringify(b.arguments)}`
              : b.type === "thinking"
                ? b.thinking
                : ""),
        )
        .join("\n");
}
function previewText(text: string, budget = 1400) {
  let result = text;
  while (Buffer.byteLength(JSON.stringify(result)) > budget)
    result = result.slice(0, Math.floor(result.length * 0.75));
  return result;
}
function snapshot(): BridgeSnapshot {
  const messages = session?.agent.state.messages ?? [];
  return {
    version,
    scope: { ...scope },
    revision: String(ledger.revision),
    watermark: String(ledger.revision),
    workspace_path: root,
    status: fatal ? "failed" : pending.size ? "awaiting_input" : active ? "running" : "idle",
    active_operation: active,
    messages: messages
      .slice(-20)
      .map((m: any, i: number) => ({
        id: String(messages.length - Math.min(messages.length, 20) + i),
        role: m.role,
        text: previewText(textOf(m)),
        evidence: evidence(m),
      })),
    message_count: messages.length,
    pending: [...pending.values()].map((p) => p.view),
    model: modelId,
    diagnostic,
    session_file: manager?.getSessionFile() ?? null,
    fixture_provider: fixture === "true",
  };
}
function publish() {
  if (fatal) return;
  latestSnapshot = snapshot();
  send({ kind: "snapshot", snapshot: latestSnapshot });
}
let scheduled: ReturnType<typeof setTimeout> | undefined;
let progressScheduled: ReturnType<typeof setTimeout> | undefined;
function progress() {
  if (progressScheduled) return;
  progressScheduled = setTimeout(() => {
    progressScheduled = undefined;
    if (active && !fatal)
      try {
        send({ kind: "progress", scope: { ...scope }, operation: active, text: streamText });
      } catch (e) {
        fail(e);
      }
  }, 16);
}
function schedule() {
  if (!scheduled)
    scheduled = setTimeout(() => {
      scheduled = undefined;
      try {
        publish();
      } catch (e) {
        fail(e);
      }
    }, 32);
}
function fail(error: unknown) {
  fatal = true;
  diagnostic = String(error);
  for (const p of pending.values()) {
    clearTimeout(p.timer);
    p.resolve(false);
  }
  pending.clear();
  void session?.abort();
  try {
    send({ kind: "fatal", detail: diagnostic });
  } catch {}
  process.exitCode = 1;
}
function persistPending() {
  atomic(
    join(state, "pending.json"),
    [...pending.values()].map((p) => p.view),
  );
}
async function ask(
  title: string,
  kind: string,
  args: unknown,
  options: string[] = [],
  signal?: AbortSignal,
): Promise<any> {
  if (pending.size >= 16) throw new Error("Too many pending questions");
  if (signal?.aborted) return false;
  const binding: ApprovalBinding = {
    id: randomUUID(),
    operation: active ?? "extension",
    argument_sha256: digest(args),
    scope: { ...scope },
    revision: String(ledger.revision),
    expires_ms: String(Date.now() + 5 * 60 * 1000),
  };
  if (options.length > 30) throw new Error("Question exceeds the bridge option limit");
  const view = {
    binding,
    title: previewText(title, 3000),
    kind,
    options,
    evidence: evidence({ title, args }),
    truncated: previewText(title, 3000) !== title,
  };
  if (
    Buffer.byteLength(JSON.stringify([...pending.values()].map((p) => p.view).concat(view))) > 24000
  )
    throw new Error("Pending review capacity reached; requests must be reviewed sequentially");
  return new Promise((done, reject) => {
    const settle = (value: any) => {
      const p = pending.get(binding.id);
      if (!p) return;
      clearTimeout(p.timer);
      pending.delete(binding.id);
      try {
        persistPending();
        done(value);
        schedule();
      } catch (e) {
        reject(e);
        fail(e);
      }
    };
    const timer = setTimeout(() => settle(false), 5 * 60 * 1000);
    pending.set(binding.id, { view, resolve: settle, timer });
    signal?.addEventListener("abort", () => settle(false), { once: true });
    try {
      persistPending();
      publish();
    } catch (e) {
      pending.delete(binding.id);
      clearTimeout(timer);
      reject(e);
    }
  });
}
const fixtureModel = {
  provider: "fixture",
  id: "fixture",
  name: "Fixture provider",
  api: "openai-completions",
  baseUrl: "http://127.0.0.1:1",
  contextWindow: 200000,
  maxTokens: 4096,
  input: ["text"],
  reasoning: false,
  cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
};
let fixtureCalls = 0;
function fakeStream(_model: any, context: any, options: any = {}) {
  const stream = new AssistantMessageEventStream();
  fixtureCalls++;
  const latest = [...context.messages].reverse().find((m: any) => m.role === "user");
  const instruction = textOf(latest ?? {}).split("\n")[0];
  const last = context.messages.at(-1);
  let content: any[];
  if (last?.role === "toolResult")
    content = [{ type: "text", text: `Tool completed. ${textOf(last).slice(0, 300)}` }];
  else if (instruction.includes("[tool]"))
    content = [
      { type: "toolCall", id: randomUUID(), name: "read", arguments: { path: "README.md" } },
    ];
  else if (instruction.includes("[approval]") || instruction.includes("[large-approval]"))
    content = [
      {
        type: "toolCall",
        id: randomUUID(),
        name: "write",
        arguments: {
          path: "approved.txt",
          content: instruction.includes("[large-approval]")
            ? "Exact approved fixture.\n".repeat(3000)
            : "Approved fixture mutation\n",
        },
      },
    ];
  else if (instruction.includes("[question]"))
    content = [
      {
        type: "toolCall",
        id: randomUUID(),
        name: "ask_user",
        arguments: { question: "Choose the fixture direction" },
      },
    ];
  else
    content = [
      {
        type: "text",
        text: instruction.includes("[stream]")
          ? Array.from(
              { length: 2000 },
              (_, i) => `Fixture event ${i}: preserved durable response.\n`,
            ).join("")
          : "AG1 fixture response: " + instruction.split("\n")[0],
      },
    ];
  let done = false;
  const timers: ReturnType<typeof setTimeout>[] = [];
  const message = (blocks: any[], reason: string) => ({
    role: "assistant",
    content: blocks,
    api: fixtureModel.api,
    provider: "fixture",
    model: "fixture",
    usage: {
      input: 0,
      output: 0,
      cacheRead: 0,
      cacheWrite: 0,
      totalTokens: 0,
      cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 },
    },
    timestamp: Date.now(),
    stopReason: reason,
  });
  const finish = () => {
    if (done) return;
    done = true;
    for (const timer of timers) clearTimeout(timer);
    const aborted = options.signal?.aborted;
    const reason = aborted ? "aborted" : content[0].type === "toolCall" ? "toolUse" : "stop";
    const final = message(aborted ? [] : content, reason);
    stream.push({
      type: aborted ? "error" : "done",
      reason,
      ...(aborted ? { error: final } : { message: final }),
    });
    stream.end();
  };
  if (instruction.includes("[stream]") && content[0].type === "text") {
    const text = content[0].text;
    let offset = 0;
    stream.push({ type: "start", partial: message([{ type: "text", text: "" }], "stop") });
    const pump = () => {
      if (done) return;
      if (options.signal?.aborted) {
        finish();
        return;
      }
      const delta = text.slice(offset, offset + 1024);
      offset += delta.length;
      stream.push({
        type: "text_delta",
        contentIndex: 0,
        delta,
        partial: message([{ type: "text", text: text.slice(0, offset) }], "stop"),
      });
      if (offset < text.length) timers.push(setTimeout(pump, 5));
      else finish();
    };
    timers.push(setTimeout(pump, 5));
  } else timers.push(setTimeout(finish, instruction.includes("[slow]") ? 1500 : 30));
  options.signal?.addEventListener("abort", finish, { once: true });
  return stream;
}
async function create(file?: string) {
  manager = file
    ? SessionManager.open(file, join(state, "sessions"))
    : SessionManager.create(root, join(state, "sessions"));
  if (file && fs.existsSync(activePath)) {
    const saved = JSON.parse(fs.readFileSync(activePath, "utf8"));
    if (saved.file === file && saved.journal_sha256 === digest(fs.readFileSync(file).toString("utf8"))) {
      if (saved.leaf === null) manager.resetLeaf();
      else if (typeof saved.leaf === "string") manager.branch(saved.leaf);
    }
  }
  scope = { ...scope, session: manager.getSessionId(), epoch: String(BigInt(scope.epoch) + 1n) };
  const auth = AuthStorage.create(join(state, "agent", "auth.json"));
  if (fixture === "true") auth.setRuntimeApiKey("fixture", "fixture");
  else auth.setRuntimeApiKey("llamacpp", "local");
  const registry = ModelRegistry.create(auth, join(state, "agent", "models.json"));
  const settings = SettingsManager.create(root, join(state, "agent"));
  const p = config.models[modelId];
  if (!p) throw new Error("Selected profile missing");
  const provider = llama.buildLlamaCppProviderConfig(
    "http://127.0.0.1:8080",
    [{ id: modelId, name: modelId, path: p.path, sizeBytes: 1, reasoning: true }],
    p.profile.contextWindow,
    {
      profiles: Object.fromEntries(
        Object.entries(config.models).map(([id, x]: any) => [id, x.profile]),
      ),
      activeModelId: modelId,
      activeContextWindow: p.profile.contextWindow,
    },
  );
  const model =
    fixture === "true"
      ? fixtureModel
      : {
          ...provider.models[0],
          provider: "llamacpp",
          api: provider.api,
          baseUrl: provider.baseUrl,
        };
  const resources = {
    getExtensions: () => ({ extensions: [], errors: [], runtime: createExtensionRuntime() }),
    getSkills: () => ({ skills: [], diagnostics: [] }),
    getPrompts: () => ({ prompts: [], diagnostics: [] }),
    getThemes: () => ({ themes: [], diagnostics: [] }),
    getAgentsFiles: () => ({ agentsFiles: [] }),
    getSystemPrompt: () => undefined,
    getAppendSystemPrompt: () => [],
    extendResources() {},
    reload: async () => {},
  };
  let approvedPolicyRequest: string | undefined;
  const safety = {
    mode: "interactive",
    cwd: root,
    trustedTools: ["ask_user"],
    approvalHandler: async (request: any) => {
      if (approvedPolicyRequest === digest(request)) {
        approvedPolicyRequest = undefined;
        return true;
      }
      return ask(request.summary + "\n" + request.details, "approval", request);
    },
  };
  const result = await createAgentSession({
    cwd: root,
    agentDir: join(state, "agent"),
    authStorage: auth,
    modelRegistry: registry,
    settingsManager: settings,
    sessionManager: manager,
    model,
    ...(fixture === "true" ? { resourceLoader: resources } : {}),
    disableWebVisualTools: true,
    toolSafety: safety,
    customTools: [
      {
        name: "ask_user",
        label: "Ask user",
        description: "Ask the user a question and wait for their answer.",
        parameters: {
          type: "object",
          properties: { question: { type: "string" } },
          required: ["question"],
        },
        execute: async (_id: string, args: any, signal: AbortSignal) => ({
          content: [
            { type: "text", text: String(await ask(args.question, "input", args, [], signal)) },
          ],
          details: {},
        }),
      },
    ],
  });
  session = result.session;
  if (fixture === "true") session.agent.streamFn = fakeStream;
  session.agent.toolExecution = "sequential";
  const previous = session.agent.beforeToolCall;
  session.agent.beforeToolCall = async (context: any, signal: AbortSignal) => {
    const name = context.toolCall.name;
    approvedPolicyRequest = undefined;
    const decision = evaluateToolSafety(name, context.args, safety);
    if (!decision.allow) return { block: true, reason: decision.reason };
    if (
      !["read", "grep", "find", "ls", "repo", "sourceContext", "artifact", "ask_user"].includes(
        name,
      )
    ) {
      const allowed = await ask(
        `${name}\n${JSON.stringify(context.args)}`,
        "approval",
        { tool: name, args: context.args },
        [],
        signal,
      );
      if (!allowed) return { block: true, reason: "User denied the exact tool operation" };
      if (decision.approvalRequest) approvedPolicyRequest = digest(decision.approvalRequest);
    }
    try {
      return await previous?.(context, signal);
    } finally {
      approvedPolicyRequest = undefined;
    }
  };
  await session.bindExtensions({
    uiContext: new Proxy(
      {
        select: (title: string, options: string[], opts: any) =>
          ask(title, "select", { title, options }, options, opts?.signal),
        confirm: (title: string, message: string, opts: any) =>
          ask(title + "\n" + message, "approval", { title, message }, [], opts?.signal),
        input: (title: string, placeholder: string, opts: any) =>
          ask(title, "input", { title, placeholder }, [], opts?.signal),
        notify: (message: string) => {
          diagnostic = String(message).slice(0, 2000);
          schedule();
        },
        setStatus: (_key: string, message: string) => {
          diagnostic = message?.slice(0, 2000) ?? null;
          schedule();
        },
        setWorkingMessage: (message: string) => {
          diagnostic = message?.slice(0, 2000) ?? null;
          schedule();
        },
      },
      {
        get: (target: any, name: string) =>
          name in target
            ? target[name]
            : () => {
                throw new Error(
                  "Terminal-only extension UI capability unavailable: " + String(name),
                );
              },
      },
    ),
    onError: (error: any) => {
      diagnostic = String(error).slice(0, 2000);
      schedule();
    },
  });
  session.subscribe((event: any) => {
    if (event.type === "message_update" && active) {
      const partial = event.assistantMessageEvent;
      if (partial?.delta) {
        streamText = (streamText + partial.delta).slice(-4000);
        progress();
      }
    } else schedule();
  });
  atomic(activePath, { file: manager.getSessionFile(), model: modelId,
    leaf: manager.getLeafId(), journal_sha256: digest(fs.readFileSync(manager.getSessionFile()).toString("utf8")) });
}
const initial = fs.existsSync(activePath)
  ? JSON.parse(fs.readFileSync(activePath, "utf8")).file
  : undefined;
if (initial && (dirname(initial) !== join(state, "sessions") || !initial.endsWith(".jsonl")))
  throw new Error("Invalid private session path");
await create(initial ?? undefined);
// Lost worker promises cannot be resurrected. Retain previous pending inputs as interrupted evidence.
if (fs.existsSync(join(state, "pending.json"))) {
  const old = JSON.parse(fs.readFileSync(join(state, "pending.json"), "utf8"));
  if (old.length) {
    evidence({ interruptedQuestions: old });
    diagnostic = "Pending input was interrupted by worker exit. It was not approved.";
  }
}
persistPending();
publish();
function reply(command: CommandEnvelope, status: string, data: any, durable = false) {
  if (!fatal) publish();
  send({
    kind: "reply",
    id: command.id,
    scope: { ...scope },
    revision: String(ledger.revision),
    durable,
    status,
    data,
  });
}
async function drain() {
  await running;
  await session.waitForEventLoop();
  syncTree(join(state, "sessions"));
}
function nativeFiles(command: any) {
  const result = spawnSync(nativeHost, ["--file-operation"], {
    input: JSON.stringify({ workspace: root, action: command.action, args: command.args }),
    encoding: "utf8",
    maxBuffer: 65536,
    timeout: 10000,
    env: { PATH: process.env.PATH },
  });
  if (result.status !== 0) throw new Error(result.stderr || "Native file operation failed");
  return JSON.parse(result.stdout);
}
async function execute(envelope: CommandEnvelope) {
  if (fatal) throw new Error("Worker failed; restart and inspect durable results before retrying");
  const { id, command } = envelope;
  if (envelope.version.schema !== schema || envelope.version.major !== 1)
    throw new Error("Incompatible protocol");
  if (JSON.stringify(envelope.scope) !== JSON.stringify(scope))
    throw new Error("Stale session/worker scope");
  const prior = ledger.get(id);
  const fingerprint = digest(envelope.command);
  if (prior) {
    if (prior.migration_receipts)
      throw new Error("Command ID is reserved by native recovery receipts; inspect command_result. No replay is allowed.");
    if (prior.fingerprint !== fingerprint) throw new Error("Command ID conflict");
    reply(envelope, prior.status, prior, true);
    return;
  }
  if (command.kind === "command_result") {
    reply(envelope, "result", ledger.get(command.command) ?? null, true);
    return;
  }
  if (command.kind === "runtime" && command.action === "migration_status") {
    const status = JSON.parse(fs.readFileSync(join(state, "native-recovery/status.json"), "utf8"));
    const rows = JSON.parse(fs.readFileSync(join(state, "native-recovery/receipts.json"), "utf8"));
    const offset = Number(command.args.offset ?? 0), limit = Number(command.args.limit ?? 50);
    if (!Number.isSafeInteger(offset) || offset < 0 || offset > rows.length || !Number.isInteger(limit) || limit < 1 || limit > 100)
      throw new Error("Invalid native recovery page");
    reply(envelope, "query", { ...status, total: rows.length, receipts: rows.slice(offset, offset + limit),
      next: offset + limit < rows.length ? String(offset + limit) : null }, false);
    return;
  }
  if (command.kind === "runtime" && command.action === "migration_receipt") {
    const key = command.args.key;
    if (typeof key !== "string" || !/^[a-zA-Z0-9_.:-]{1,128}$/.test(key)) throw new Error("Invalid native receipt key");
    const receipt = JSON.parse(fs.readFileSync(join(state, "native-recovery/receipts", digest(key) + ".json"), "utf8"));
    if (receipt.id !== key || receipt.replay_allowed !== false) throw new Error("Invalid native receipt");
    const decode = (bytes: number[] | null) => {
      if (bytes === null) return null;
      const raw = Buffer.from(bytes);
      try { return JSON.parse(raw.toString("utf8")); } catch { return { base64: raw.toString("base64") }; }
    };
    reply(envelope, "query", { key, state: receipt.state, replay_allowed: false,
      evidence: evidence({ ...receipt, intent: decode(receipt.intent), result: decode(receipt.result) }) }, false);
    return;
  }
  if (command.kind === "session" && command.action === "snapshot") {
    publish();
    reply(envelope, "snapshot", null);
    return;
  }
  if (command.kind === "artifact_read") {
    if (command.limit < 1 || command.limit > 32768) throw new Error("Invalid page limit");
    const id = command.artifact;
    if (!/^[0-9a-f]{64}$/.test(id)) throw new Error("Unknown evidence ID");
    const path = join(state, "evidence", id + ".json");
    const fd = fs.openSync(path, "r");
    try {
      const size = fs.fstatSync(fd).size;
      const start = Number(command.offset);
      if (!Number.isSafeInteger(start) || start < 0 || start > size)
        throw new Error("Invalid offset");
      const buffer = Buffer.alloc(Math.min(command.limit, size - start));
      const count = fs.readSync(fd, buffer, 0, buffer.length, start);
      const page = buffer.subarray(0, count);
      reply(envelope, "page", {
        base64: page.toString("base64"),
        next: start + count < size ? String(start + count) : null,
      });
    } finally {
      fs.closeSync(fd);
    }
    return;
  }
  if (command.kind === "approve" || (command.kind === "session" && command.action === "answer")) {
    const args: any = command.kind === "approve" ? command : command.args;
    const p = pending.get(args.binding.id);
    if (
      !p ||
      JSON.stringify(p.view.binding) !== JSON.stringify(args.binding) ||
      Date.now() > Number(p.view.binding.expires_ms)
    )
      throw new Error("Stale or invalid approval binding");
    if (p.view.kind === "select" && args.allow !== false && !p.view.options.includes(args.answer))
      throw new Error("Answer must match an offered option");
    ledger.save(id, {
      fingerprint,
      status: "completed",
      binding: args.binding,
      answer: args.allow ?? args.answer,
    });
    p.resolve(args.allow ?? args.answer);
    reply(envelope, "completed", null, true);
    publish();
    return;
  }
  if (command.kind === "cancel") {
    if (command.operation !== active) throw new Error("Stale operation");
    ledger.save(id, { fingerprint, status: "intent" });
    for (const p of pending.values()) p.resolve(false);
    await session.abort();
    await drain();
    ledger.save(id, { fingerprint, status: "completed" });
    reply(envelope, "completed", null, true);
    publish();
    return;
  }
  // Read-only projections never advance the session revision and remain available during a turn.
  let queryData: any;
  let isQuery = false;
  if (command.kind === "model" && command.action === "status") {
    isQuery = true;
    queryData = {
      models: Object.entries(config.models).map(([id, row]: any) => ({ id, profile: row.profile })),
      tuning: false,
      automaticEviction: false,
      fixtureCalls,
    };
  } else if (command.kind === "session" && command.action === "list") {
    isQuery = true;
    queryData = fs
      .readdirSync(join(state, "sessions"))
      .filter((n) => n.endsWith(".jsonl"))
      .map((n) => join(state, "sessions", n));
  } else if (command.kind === "files" && ["list", "read", "diff"].includes(command.action)) {
    isQuery = true;
    queryData = nativeFiles(command);
  } else if (
    ["task", "memory", "artifacts"].includes(command.kind) &&
    "action" in command &&
    command.action === "list"
  ) {
    isQuery = true;
    if (command.kind === "artifacts") queryData = session.artifactStore.list(50);
    else {
      const files = fs
        .readdirSync(join(state, "project"))
        .filter((n) => n.includes(command.kind) && n.endsWith(".json"));
      queryData = files.map((file) => ({
        file,
        evidence: evidence(JSON.parse(fs.readFileSync(join(state, "project", file), "utf8"))),
      }));
    }
  }
  if (isQuery) {
    reply(envelope, "query", queryData, false);
    return;
  }
  if (envelope.expected_revision !== String(ledger.revision))
    throw new Error("Stale revision; refresh before retrying");
  if (active && !(command.kind === "session" && command.action === "steer"))
    throw new Error("Session busy; only steering/cancel/input are admitted");
  if (command.kind === "prompt") {
    if (command.input.kind !== "inline" || !command.input.text.trim())
      throw new Error("Inline prompt required");
    ledger.save(id, { fingerprint, status: "intent" });
    active = randomUUID();
    const operation = active;
    ledger.save(id, { fingerprint, status: "accepted", operation });
    reply(envelope, "accepted", { operation }, true);
    publish();
    running = (async () => {
      try {
        if (fixture !== "true") {
          const p = config.models[modelId];
          const outcome = await llama.ensureLlamaCppServer({
            agentDir: join(state, "agent"),
            preferredModelId: modelId,
            searchDirs: [dirname(p.path)],
            env: { PATH: process.env.PATH, AG1_LLAMA_CPP: "1" },
            waitReadyMs: 180000,
            vramTune: false,
          });
          if (!outcome.ready) throw new Error(outcome.reason ?? "Preserved model is not ready");
        }
        const addendum = await buildTurnSystemPromptAddendum({
          text: command.input.kind === "inline" ? command.input.text : "",
          cwd: root,
          includeOrientation: true,
          includeFinishedJobs: true,
        });
        await session.prompt(command.input.kind === "inline" ? command.input.text : "", {
          source: "rpc",
          systemPromptAddendum: addendum,
        });
        await session.waitForEventLoop();
        syncTree(join(state, "sessions"));
        syncTree(join(state, "project"));
        const end = session.agent.state.messages.at(-1);
        ledger.save(id, {
          fingerprint,
          status:
            end?.stopReason === "aborted"
              ? "cancelled"
              : end?.stopReason === "error"
                ? "failed"
                : "completed",
          operation,
        });
      } catch (e) {
        diagnostic = String(e);
        try {
          ledger.save(id, { fingerprint, status: "failed", operation, detail: diagnostic });
        } catch (persistence) {
          fail(persistence);
        }
      } finally {
        if (progressScheduled) clearTimeout(progressScheduled);
        progressScheduled = undefined;
        active = null;
        streamText = "";
        running = undefined;
        if (!fatal) publish();
      }
    })();
    return;
  }
  ledger.save(id, { fingerprint, status: "intent" });
  let data: any = null;
  try {
    if (command.kind === "session") {
      if (command.action === "steer") {
        const args: any = command.args;
        if (!active || session.getSteeringMessages().length >= 256)
          throw new Error("No active session or steering queue full");
        await session.steer(String(args.text));
      } else if (command.action === "new") {
        await drain();
        session.dispose();
        await create();
      } else if (command.action === "fork") {
        await drain();
        const leaf = (command.args as any).leaf ?? manager.getLeafId();
        manager.createBranchedSession(leaf);
        const file = manager.getSessionFile();
        session.dispose();
        await create(file);
      } else if (command.action === "resume") {
        await drain();
        const name = (command.args as any).file;
        if (
          typeof name !== "string" ||
          dirname(name) !== join(state, "sessions") ||
          !name.endsWith(".jsonl")
        )
          throw new Error("Session outside private destination");
        session.dispose();
        await create(name);
      } else throw new Error("Unsupported session action");
    } else if (command.kind === "model" && command.action === "select") {
      await drain();
      const next = (command.args as any).id;
      if (!config.models[next]) throw new Error("Unpinned model");
      modelId = next;
      session.dispose();
      await create(manager.getSessionFile());
    } else if (command.kind === "files" && command.action === "save") {
      data = nativeFiles(command);
    } else throw new Error("Capability unavailable in this bridge");
    syncTree(join(state, "sessions"));
    ledger.save(id, { fingerprint, status: "completed", data });
    reply(envelope, "completed", data, true);
    publish();
  } catch (e) {
    ledger.save(id, { fingerprint, status: "failed", detail: String(e) });
    throw e;
  }
}
let buffer = "";
process.stdin.setEncoding("utf8");
process.stdin.on("data", (chunk: string) => {
  buffer += chunk;
  if (Buffer.byteLength(buffer) > 131072) {
    fail("Input framing exceeded limit");
    process.stdin.destroy();
    return;
  }
  while (buffer.includes("\n")) {
    const i = buffer.indexOf("\n");
    const line = buffer.slice(0, i);
    buffer = buffer.slice(i + 1);
    if (Buffer.byteLength(line) > 65536 || queued >= 256) {
      fail("Bounded command queue exceeded");
      process.stdin.destroy();
      return;
    }
    queued++;
    serial = serial
      .then(async () => {
        let envelope: CommandEnvelope | undefined;
        try {
          const wire = JSON.parse(line);
          if (wire.type !== "command") throw new Error("Command envelope required");
          envelope = wire.data;
          await execute(envelope!);
        } catch (e) {
          if (envelope) reply(envelope!, "rejected", { detail: String(e) });
          else fail(e);
        } finally {
          queued--;
        }
      })
      .catch(fail);
  }
});
process.stdin.on("end", () => {
  void (async () => {
    for (const p of pending.values()) p.resolve(false);
    await session.abort();
    await drain();
    session.dispose();
    if (scheduled) clearTimeout(scheduled);
    process.exit(fatal ? 1 : 0);
  })().catch(fail);
});
