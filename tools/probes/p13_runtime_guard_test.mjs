import assert from "node:assert/strict";
import { runtimeGuard } from "../../product/compat/agent-worker/runtime.ts";
import { buildLlamaCppServerArgs, resolveLlamaCppPort, resolveLlamaCppHost,
  resolveLlamaCppContextBatchTier } from "../../reference/ag1-executable/packages/coding-agent/dist/core/llamacpp-runtime.js";

const profile = { serverPath: "/fixture/preserved-server", libPath: "/fixture/preserved-libraries",
  contextWindow: 90000, nCpuMoe: 0, batchSize: 256, ubatchSize: 128,
  priority: false, slotSimilarityFlag: "none", extraArgs: ["--ordered", "3", "1", "2"],
  contextBatchTiers: [{ maxContextWindow: 200000, batchSize: 512, ubatchSize: 256 },
    { maxContextWindow: 90000, batchSize: 1024, ubatchSize: 1024 }],
};
const config = { models: { fixture: { path: "/fixture/weights.gguf", profile } }, material: [] };
const guard = runtimeGuard(config, "/fixture/raw-profile", buildLlamaCppServerArgs);
for (const env of [{}, { AG1_LLAMA_CPP_PORT: "19001", AG1_LLAMA_CPP_HOST: "localhost" },
  { AG1_LLAMA_CPP_PORT: "19001", LLAMA_CPP_PORT: "19002", AG1_LLAMA_CPP_HOST: "localhost", LLAMA_CPP_HOST: "127.0.0.1" },
  { LLAMA_CPP_PORT: "invalid", AG1_LLAMA_CPP_PORT: "19001" }]) {
  const tier = resolveLlamaCppContextBatchTier(profile, profile.contextWindow);
  const args = buildLlamaCppServerArgs({ ...profile, modelPath: config.models.fixture.path,
    alias: "fixture", host: resolveLlamaCppHost(env), port: resolveLlamaCppPort(env),
    batchSize: tier.batchSize, ubatchSize: tier.ubatchSize });
  const plan = { model: { id: "fixture", path: config.models.fixture.path }, profile,
    binary: { command: profile.serverPath, libPath: profile.libPath }, args, env };
  await guard.checkPlan(plan);
  const changed = structuredClone(plan);
  changed.args[changed.args.indexOf("--port") + 1] = "20000";
  await assert.rejects(guard.checkPlan(changed), /Launch argv differs/);
  changed.args = [...args.slice(0, -3), "1", "3", "2"];
  await assert.rejects(guard.checkPlan(changed), /Launch argv differs/);
}
console.log("Four TS guard/default/precedence/tier cases passed; no model or network action.");
