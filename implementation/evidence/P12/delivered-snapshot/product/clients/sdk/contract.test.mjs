import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import * as sdk from "./dist/index.js";
test("standalone ESM facade imports without React, Tauri or the TS agent loop", () => {
  assert.equal(typeof sdk.NativeClient, "function");
  assert.equal(typeof sdk.createAgentSession, "function");
  assert.throws(() => sdk.getAgentDir(), sdk.UnsupportedAPI);
  assert.throws(() => sdk.takeOverStdout(), sdk.UnsupportedAPI);
  const code = readFileSync(new URL("./dist/index.js", import.meta.url), "utf8");
  assert.ok(!/from ["'](?:react|@tauri|@ag1\/coding-agent)/.test(code));
});
test("the dead hooks subpath fails explicitly without inventing hook symbols", async () => {
  await assert.rejects(import("./dist/hooks.js"), error => error instanceof sdk.UnsupportedAPI && error.message.includes("targets are absent"));
});
