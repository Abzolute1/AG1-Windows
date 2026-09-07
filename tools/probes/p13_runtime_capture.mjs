// Read-only resolution through the locked TS implementation. Every operation
// that could contact/control a model is replaced before the resolver is called.
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import { join, resolve } from "node:path";
import { pathToFileURL } from "node:url";

const [reference, requestFile, outputFile] = process.argv.slice(2).map(path => resolve(path));
const request = JSON.parse(fs.readFileSync(requestFile, "utf8"));
const runtime = await import(pathToFileURL(join(reference, "packages/coding-agent/dist/core/llamacpp-runtime.js")));
const scratch = fs.mkdtempSync(join(os.tmpdir(), "ag1-p13-resolve-"));
fs.chmodSync(scratch, 0o700);
const captures = [];
try {
  for (const [id, path] of Object.entries(request.models)) {
    const agentDir = join(scratch, String(captures.length));
    fs.mkdirSync(agentDir, { mode: 0o700 });
    fs.copyFileSync(request.profile, runtime.modelProfilesPath(agentDir));
    fs.chmodSync(runtime.modelProfilesPath(agentDir), 0o600);
    const launches = [];
    let healthCalls = 0;
    const result = await runtime.ensureLlamaCppServer({
      agentDir, env: request.environment, searchDirs: [], preferredModelId: path,
      waitReadyMs: 0, vramTune: false,
      deps: {
        env: request.environment, homedir: agentDir, vramMiB: request.vram_mib,
        now: () => 1000,
        fetchImpl: async () => { healthCalls++; return new Response("", { status: 503 }); },
        spawnImpl: (command, args, options) => {
          launches.push({ command, args, environment: options.env });
          return { pid: 987654, unref() {} };
        },
        pidAlive: () => false, listListenPids: () => [],
        kill: () => { throw new Error("Capture must never signal a process"); },
        sleep: async () => { throw new Error("Capture must never enter load/tuning"); },
      },
    });
    assert.equal(launches.length, 1);
    assert.equal(result.model.id, id);
    assert.equal(result.started, true);
    assert.equal(result.ready, false);
    const model = result.providerConfig.models.find(model => model.id === id);
    assert(model);
    captures.push({ model_id: id, model_path: path, ...launches[0],
      context_window: model.contextWindow, max_tokens: model.maxTokens,
      provider_model: { ...model, provider: "llamacpp", api: result.providerConfig.api,
        baseUrl: result.providerConfig.baseUrl },
      fake_health_calls: healthCalls,
    });
  }
  fs.writeFileSync(outputFile, JSON.stringify({ version: 1, captures,
    model_actions: { spawn: 0, stop: 0, network: 0, tune: 0 },
    scope: "Pinned TS resolver with actual read-only model paths and private environment; fake process/HTTP dependencies.",
  }), { flag: "wx", mode: 0o600 });
  console.log(JSON.stringify({ models: captures.length, model_actions: 0 }));
} finally {
  fs.rmSync(scratch, { recursive: true, force: true });
}
