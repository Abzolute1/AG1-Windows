import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { fileURLToPath, pathToFileURL } from "node:url";
await import("../offline_guard.mjs");
const root = path.resolve(
	path.dirname(fileURLToPath(import.meta.url)),
	"../..",
);
const { parseStreamingJson, parseFinalToolCallJson } = await import(
	pathToFileURL(
		path.join(
			root,
			"reference/ag1-executable/packages/ai/dist/utils/json-parse.js",
		),
	)
);
const inputs = new Set([
	"",
	" ",
	"garbage",
	'{"x":',
	'{"x":1, broken',
	'{"x" 12',
	"[1,,2]",
	'{"a":1 "b":2}',
	'{"__proto__":{"fixture":true},"constructor":1,',
	"1E",
	"1e",
	"1.",
	"-",
	"true trailing",
	"null trailing",
	"[NaN,",
	'{"x":Inf',
	'"bad\\q',
	'"bad\\u12',
	'"bad\\\\',
	'"escaped\\\"quote',
]);
for (const document of [
	'{"path":"file.txt","line":12,"enabled":true,"missing":null,"disabled":false}',
	'{"text":"🦙 café\\nline\\tend","nested":[{"a":1},2,-3.5,4e+2]}',
	'{"q":"quote\\\" and slash\\\\ and unicode\\u0061"}',
	'{"one":[[[],{}]],"empty":"","exponent":1e-12}',
	"[1,2,3]",
	"null",
	"true",
	"false",
	"123e+4",
	"123E-4",
]) {
	let prefix = "";
	for (const char of document) {
		prefix += char;
		inputs.add(prefix);
	}
}
function boundary(value) {
	if (typeof value === "number" && !Number.isFinite(value)) return "nonfinite";
	if (typeof value === "string" && !value.isWellFormed()) return "surrogate";
	if (value && typeof value === "object")
		for (const child of Object.values(value)) {
			const result = boundary(child);
			if (result) return result;
		}
	return null;
}
const rows = [...inputs].map((input) => {
	const preview = parseStreamingJson(input),
		final = parseFinalToolCallJson(input);
	return {
		input,
		boundary: boundary(preview),
		preview: JSON.parse(JSON.stringify(preview)),
		final,
	};
});
const result = {
	version: 1,
	source_commit: "e30f0b4b238ee5618928bcd17b4571eb046e7038",
	dependency: "partial-json@0.1.7",
	rows,
};
const target = path.join(root, "fixtures/tools/partial-json-oracle.json");
if (process.argv.includes("--capture")) {
	assert(!fs.existsSync(target));
	fs.writeFileSync(target, JSON.stringify(result, null, 2) + "\n");
} else assert.deepEqual(result, JSON.parse(fs.readFileSync(target, "utf8")));
console.log(
	JSON.stringify({
		status: "passed",
		partial_json_cases: rows.length,
		network_actions: 0,
		model_actions: 0,
	}),
);
