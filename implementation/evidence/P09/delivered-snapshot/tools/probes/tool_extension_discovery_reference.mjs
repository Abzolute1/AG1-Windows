import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { fileURLToPath, pathToFileURL } from "node:url";
await import("../offline_guard.mjs");
const root = path.resolve(
	path.dirname(fileURLToPath(import.meta.url)),
	"../..",
);
const { discoverAndLoadExtensions, loadExtensions } = await import(
	pathToFileURL(
		path.join(
			root,
			"reference/ag1-executable/packages/coding-agent/dist/core/extensions/loader.js",
		),
	)
);
const code = `export default api=>api.registerCommand('fixture',{handler:async()=>{}});`;
const sources = (paths) =>
	Object.fromEntries(paths.map((name) => [name, code]));
const manifest = (paths) =>
	JSON.stringify({ name: "fixture", ag1: { extensions: paths } });
const cases = [
	{
		id: "direct-ts",
		files: sources(["extensions/foo.ts", "extensions/bar.ts"]),
	},
	{ id: "direct-js", files: sources(["extensions/foo.js"]) },
	{ id: "index-ts", files: sources(["extensions/nested/index.ts"]) },
	{ id: "index-js", files: sources(["extensions/nested/index.js"]) },
	{
		id: "index-preference",
		files: sources([
			"extensions/nested/index.ts",
			"extensions/nested/index.js",
		]),
	},
	{
		id: "manifest-entry",
		files: {
			...sources(["extensions/pkg/src/main.ts"]),
			"extensions/pkg/package.json": manifest(["./src/main.ts"]),
		},
	},
	{
		id: "manifest-multiple",
		files: {
			...sources(["extensions/pkg/a.ts", "extensions/pkg/b.ts"]),
			"extensions/pkg/package.json": manifest(["./a.ts", "./b.ts"]),
		},
	},
	{
		id: "manifest-precedence",
		files: {
			...sources(["extensions/pkg/custom.ts", "extensions/pkg/index.ts"]),
			"extensions/pkg/package.json": manifest(["./custom.ts"]),
		},
	},
	{
		id: "manifest-without-ag1",
		files: {
			...sources(["extensions/pkg/index.ts"]),
			"extensions/pkg/package.json": JSON.stringify({
				name: "fixture",
				version: "1.0.0",
			}),
		},
	},
	{
		id: "no-entry",
		files: sources([
			"extensions/skipped/helper.ts",
			"extensions/skipped/utils.ts",
		]),
	},
	{ id: "one-level", files: sources(["extensions/container/nested/index.ts"]) },
	{
		id: "mixed",
		files: {
			...sources([
				"extensions/direct.ts",
				"extensions/index/index.ts",
				"extensions/pkg/entry.ts",
			]),
			"extensions/pkg/package.json": manifest(["./entry.ts"]),
		},
	},
	{
		id: "missing-declared-entry",
		files: {
			...sources(["extensions/pkg/exists.ts"]),
			"extensions/pkg/package.json": manifest(["./exists.ts", "./missing.ts"]),
		},
	},
	{
		id: "configured-path",
		files: sources(["custom/my-ext.ts"]),
		explicit: ["custom/my-ext.ts"],
	},
	{
		id: "explicit-only",
		files: sources(["extensions/discovered.ts", "explicit.ts"]),
		mode: "explicit",
		explicit: ["explicit.ts"],
	},
	{
		id: "empty-explicit",
		files: sources(["extensions/discovered.ts"]),
		mode: "explicit",
		explicit: [],
	},
];
const rows = [];
for (const input of cases) {
	const fixture = fs.mkdtempSync(
		path.join(root, ".local/p09-extension-discovery-"),
	);
	try {
		fs.mkdirSync(path.join(fixture, "extensions"));
		for (const [name, body] of Object.entries(input.files)) {
			const file = path.join(fixture, name);
			fs.mkdirSync(path.dirname(file), { recursive: true });
			fs.writeFileSync(file, body);
		}
		const explicit = (input.explicit ?? []).map((name) =>
			path.join(fixture, name),
		);
		const result =
			input.mode === "explicit"
				? await loadExtensions(explicit, fixture)
				: await discoverAndLoadExtensions(explicit, fixture, fixture);
		assert.deepEqual(result.errors, []);
		rows.push({
			input,
			output: {
				paths: result.extensions
					.map((entry) => path.relative(fixture, entry.path))
					.sort(),
				errors: [],
			},
		});
	} finally {
		fs.rmSync(fixture, { recursive: true, force: true });
	}
}
const result = {
	version: 1,
	source_commit: "e30f0b4b238ee5618928bcd17b4571eb046e7038",
	model_actions: 0,
	network_actions: 0,
	rows,
};
const target = path.join(
	root,
	"fixtures/tools/extension-discovery-oracle.json",
);
if (process.argv.includes("--capture")) {
	assert(!fs.existsSync(target));
	fs.writeFileSync(target, JSON.stringify(result, null, 2) + "\n");
} else assert.deepEqual(result, JSON.parse(fs.readFileSync(target, "utf8")));
console.log(
	JSON.stringify({
		status: "passed",
		extension_discovery_cases: rows.length,
		model_actions: 0,
		network_actions: 0,
	}),
);
