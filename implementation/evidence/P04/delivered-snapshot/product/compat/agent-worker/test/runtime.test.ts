import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import { resolve, join } from "node:path";
import { pathToFileURL } from "node:url";
import { runtimeGuard } from "../runtime.ts";
const root = resolve(import.meta.dirname, "../../../..");
const runtime: any = await import(
  pathToFileURL(
    join(root, "reference/ag1-bridge/packages/coding-agent/dist/core/llamacpp-runtime.js"),
  ).href
);
const config = JSON.parse(fs.readFileSync(join(root, ".local/bridge-runtime.json"), "utf8"));
test("all three profiles retain their binary, libraries, flags and context tiers; drift and tuning are rejected", async () => {
  const guard = runtimeGuard(config, config.profile_file, runtime.buildLlamaCppServerArgs);
  const captured = JSON.parse(
    fs.readFileSync(join(root, "fixtures/llama/launch-plans.json"), "utf8"),
  ).captures;
  for (const [id, entry] of Object.entries(config.models) as [string, any][]) {
    const plan = captured.find((x: any) => x.model === id && x.scenario === "profile-default");
    assert(plan);
    const args = plan.observed.args.map((a: string) =>
      a.replace("<fixture>/" + id + ".gguf", entry.path),
    );
    // P01 used port 18080 and an inherited library search path. Keep those fixture inputs.
    await guard.checkPlan({
      binary: { command: plan.observed.command, libPath: entry.profile.libPath },
      args,
      model: { id, path: entry.path },
      profile: entry.profile,
      env: { AG1_LLAMA_CPP_PORT: "18080" },
    });
    await assert.rejects(
      () =>
        guard.checkPlan({
          binary: { command: "/usr/bin/llama-server" },
          args,
          model: { id, path: entry.path },
          profile: entry.profile,
          env: {},
        }),
      /binary/,
    );
    await assert.rejects(
      () =>
        guard.checkPlan({
          binary: { command: entry.profile.serverPath, libPath: entry.profile.libPath },
          args: [...args, "--fit", "on"],
          model: { id, path: entry.path },
          profile: entry.profile,
          env: {},
        }),
      /argv/,
    );
  }
  assert.throws(() => guard.checkOptions({ vramTune: true }), /Tuning/);
  assert.throws(() => guard.checkOptions({ forcedNCpuMoe: 7 }), /Tuning/);
  assert.throws(
    () => guard.checkOptions({ env: { AG1_LLAMA_SERVER: "/usr/bin/llama-server" } }),
    /override/,
  );
  assert.equal(guard.owns(process.pid), false);
  assert.equal(guard.owns(1), false);
});
test("resolver cannot evict foreign listeners or act without an installed owner", async () => {
  let killed = 0;
  assert.throws(
    () => runtime.evictLlamaCppPort(18080, { listListenPids: () => [12345], kill: () => killed++ }),
    /conflict/,
  );
  assert.equal(killed, 0);
  await assert.rejects(
    () => runtime.ensureLlamaCppServer({ agentDir: "/fixture", env: { AG1_LLAMA_CPP: "1" } }),
    /no installed owner/,
  );
});
test("ownership lasts only for a live child with the adopted kernel identity", async () => {
  const { spawn } = await import("node:child_process");
  const once = (child: any, event: string) =>
    new Promise<void>((resolve) => child.once(event, () => resolve()));
  const guard = runtimeGuard(config, config.profile_file, runtime.buildLlamaCppServerArgs);
  const child = spawn(process.execPath, ["-e", "process.stdin.resume()"], {
    stdio: ["pipe", "ignore", "ignore"],
  });
  await once(child, "spawn");
  try {
    guard.adopt(child);
    assert(guard.owns(child.pid!));
    assert.equal(guard.owns(process.pid), false);
    child.stdin!.end();
    await once(child, "exit");
    assert.equal(guard.owns(child.pid!), false);
  } finally {
    if (child.exitCode === null && child.signalCode === null) child.kill();
  }
});
