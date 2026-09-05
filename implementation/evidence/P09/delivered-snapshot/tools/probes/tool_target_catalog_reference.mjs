import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { fileURLToPath, pathToFileURL } from "node:url";
await import("../offline_guard.mjs");
const root = path.resolve(
	path.dirname(fileURLToPath(import.meta.url)),
	"../..",
);
const visual = await import(
	pathToFileURL(
		path.join(
			root,
			"reference/ag1-executable/packages/coding-agent/dist/core/tools/visual/index.js",
		),
	)
);
const definitions = [
	visual.createScreenshotUrlToolDefinition(),
	visual.createInteractThenScreenshotToolDefinition(),
	visual.createVerifyUiToolDefinition({}),
];
const catalog = definitions.map(
	({
		name,
		label,
		description,
		parameters,
		promptSnippet,
		promptGuidelines,
	}) => ({
		name,
		label,
		description,
		parameters,
		promptSnippet,
		promptGuidelines,
	}),
);
const output = JSON.parse(
	JSON.stringify({
		version: 1,
		source_commit: "e30f0b4b238ee5618928bcd17b4571eb046e7038",
		catalog,
	}),
);
const target = path.join(root, "fixtures/tools/target-catalog-oracle.json");
const resource = path.join(
	root,
	"product/crates/ag1-core/resources/target-tool-catalog.json",
);
if (process.argv.includes("--capture")) {
	assert(!fs.existsSync(target));
	assert(!fs.existsSync(resource));
	fs.writeFileSync(target, JSON.stringify(output, null, 2) + "\n");
	fs.writeFileSync(resource, JSON.stringify(output.catalog, null, 2) + "\n");
} else {
	assert.deepEqual(output, JSON.parse(fs.readFileSync(target, "utf8")));
	assert.deepEqual(
		output.catalog,
		JSON.parse(fs.readFileSync(resource, "utf8")),
	);
}
console.log(
	JSON.stringify({
		status: "passed",
		definitions: catalog.length,
		model_actions: 0,
		browser_actions: 0,
		network_actions: 0,
	}),
);
