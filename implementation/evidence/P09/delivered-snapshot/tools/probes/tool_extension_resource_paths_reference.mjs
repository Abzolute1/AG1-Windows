import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { fileURLToPath, pathToFileURL } from "node:url";
await import("../offline_guard.mjs");
const root = path.resolve(
	path.dirname(fileURLToPath(import.meta.url)),
	"../..",
);
const { AgentSessionExtensionLifecycleCoordinator } = await import(
	pathToFileURL(
		path.join(
			root,
			"reference/ag1-executable/packages/coding-agent/dist/core/agent-session-extension-lifecycle-coordinator.js",
		),
	)
);
// The metadata conversion is pure. No session, loader or event lifecycle is started.
const coordinator = new AgentSessionExtensionLifecycleCoordinator(
	Object.freeze({}),
);
const extensionPath = "/fixture/project/.ag1/extensions/fixture.ts";
const input = {
	skillPaths: [{ path: "../contrib/skill", extensionPath }],
	promptPaths: [{ path: "../contrib/prompts", extensionPath }],
	themePaths: [{ path: "../contrib/theme.json", extensionPath }],
};
const output = Object.fromEntries(
	Object.entries(input).map(([kind, entries]) => [
		kind,
		coordinator.buildExtensionResourcePaths(entries),
	]),
);
const result = {
	version: 1,
	source_commit: "e30f0b4b238ee5618928bcd17b4571eb046e7038",
	input,
	output,
};
const target = path.join(
	root,
	"fixtures/tools/extension-resource-paths-oracle.json",
);
if (process.argv.includes("--capture")) {
	assert(!fs.existsSync(target));
	fs.writeFileSync(target, `${JSON.stringify(result, null, 2)}\n`);
} else assert.deepEqual(result, JSON.parse(fs.readFileSync(target, "utf8")));
console.log(
	JSON.stringify({
		status: "passed",
		extension_resource_path_kinds: 3,
		network_actions: 0,
		model_actions: 0,
	}),
);
