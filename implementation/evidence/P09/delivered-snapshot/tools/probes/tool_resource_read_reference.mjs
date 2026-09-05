import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { fileURLToPath, pathToFileURL } from "node:url";
await import("../offline_guard.mjs");
const root = path.resolve(
	path.dirname(fileURLToPath(import.meta.url)),
	"../..",
);
const { createReadToolDefinition } = await import(
	pathToFileURL(
		path.join(
			root,
			"reference/ag1-executable/packages/coding-agent/dist/core/tools/read.js",
		),
	)
);
const fixture = fs.mkdtempSync(path.join(root, ".local/p09-resource-read-"));
const files = {
	"skill/SKILL.md": "# Trusted skill instructions\n",
	"skill/assets/example.txt": "Referenced asset.\n",
	"unrelated/secret.txt": "not allowed\n",
};
const inputs = [
	{ path: "/fixture/skill/SKILL.md", granted: true },
	{ path: "../skill/assets/example.txt", granted: true },
	{ path: "/fixture/unrelated/secret.txt", granted: true },
	{ path: "/fixture/skill/SKILL.md", granted: false },
];
const rows = [];
try {
	fs.mkdirSync(path.join(fixture, "project"));
	for (const [name, contents] of Object.entries(files)) {
		fs.mkdirSync(path.dirname(path.join(fixture, name)), { recursive: true });
		fs.writeFileSync(path.join(fixture, name), contents);
	}
	for (const [index, input] of inputs.entries()) {
		const read = createReadToolDefinition(path.join(fixture, "project"), {
			allowedRoots: input.granted ? [path.join(fixture, "skill")] : [],
		});
		let output;
		try {
			output = {
				result: await read.execute(`resource-${index}`, {
					path: input.path.replace("/fixture/", `${fixture}/`),
				}),
			};
		} catch (error) {
			assert.match(error.message, /Path escapes the workspace/);
			output = { denied: true };
		}
		rows.push({ input, output: JSON.parse(JSON.stringify(output)) });
	}
} finally {
	fs.rmSync(fixture, { recursive: true, force: true });
}
assert(rows[0].output.result);
assert(rows[1].output.result);
assert(rows[2].output.denied);
assert(rows[3].output.denied);
const out = {
	version: 1,
	source_commit: "e30f0b4b238ee5618928bcd17b4571eb046e7038",
	model_actions: 0,
	network_actions: 0,
	files,
	rows,
};
const target = path.join(root, "fixtures/tools/resource-read-oracle.json");
if (process.argv.includes("--capture")) {
	assert(!fs.existsSync(target));
	fs.writeFileSync(target, `${JSON.stringify(out, null, 2)}\n`);
} else assert.deepEqual(out, JSON.parse(fs.readFileSync(target, "utf8")));
console.log(
	JSON.stringify({
		status: "passed",
		resource_read_cases: rows.length,
		model_actions: 0,
		network_actions: 0,
	}),
);
