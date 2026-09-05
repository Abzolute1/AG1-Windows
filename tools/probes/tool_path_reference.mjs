import fs from "node:fs";
import path from "node:path";
import os from "node:os";
import assert from "node:assert/strict";
import { fileURLToPath, pathToFileURL } from "node:url";
await import("../offline_guard.mjs");
const root = path.resolve(
	path.dirname(fileURLToPath(import.meta.url)),
	"../..",
);
const sdk = await import(
	pathToFileURL(
		path.join(
			root,
			"reference/ag1-executable/packages/coding-agent/dist/core/tools/path-utils.js",
		),
	)
);
const owned = fs.mkdtempSync(path.join(root, ".local/p09-paths-"));
const rows = [];
try {
	const cases = [
		{ name: "existing", stored: "test-file.txt", requested: "test-file.txt" },
		{ name: "nfd", stored: "filee\u0301.txt", requested: "fileé.txt" },
		{
			name: "curly",
			stored: "Capture d’cran.txt",
			requested: "Capture d'cran.txt",
		},
		{
			name: "accent-curly",
			stored: "Capture d’écran.txt",
			requested: "Capture d'écran.txt",
		},
		{
			name: "screenshot",
			stored: "Screenshot 2024-01-01 at 10.00.00\u202fAM.png",
			requested: "Screenshot 2024-01-01 at 10.00.00 AM.png",
		},
		{
			name: "nfd-curly",
			stored: "Capture d’e\u0301cran.txt",
			requested: "Capture d'écran.txt",
		},
		{
			name: "literal-precedence",
			stored: "fileé.txt",
			requested: "fileé.txt",
			other: "filee\u0301.txt",
		},
		{ name: "space", stored: "file name.txt", requested: "file\u00a0name.txt" },
	];
	for (const input of cases) {
		const cwd = path.join(owned, input.name);
		fs.mkdirSync(cwd);
		fs.writeFileSync(path.join(cwd, input.stored), input.name);
		if (input.other)
			fs.writeFileSync(path.join(cwd, input.other), "not selected");
		const output = path.relative(
			cwd,
			sdk.resolveReadPath(input.requested, cwd),
		);
		assert.equal(output, input.stored);
		rows.push({ input, output });
	}
	const cwd = path.join(owned, "repair");
	fs.mkdirSync(path.join(cwd, "nested"), { recursive: true });
	fs.writeFileSync(path.join(cwd, "nested/child.txt"), "child");
	assert.equal(
		sdk.resolveReadPath(cwd.slice(1) + "/nested/child.txt", cwd),
		path.join(cwd, "nested/child.txt"),
	);
	assert.equal(
		sdk.resolveContainedPath(cwd.slice(1) + "/nested", cwd),
		path.join(cwd, "nested"),
	);
	assert.equal(
		sdk.resolveToCwd("/absolute/path/file.txt", "/some/cwd"),
		"/absolute/path/file.txt",
	);
	assert.equal(
		sdk.resolveToCwd("relative/file.txt", "/some/cwd"),
		"/some/cwd/relative/file.txt",
	);
	assert.equal(sdk.expandPath("~"), os.homedir());
	assert.equal(
		sdk.expandPath("~/Documents/file.txt"),
		path.join(os.homedir(), "Documents/file.txt"),
	);
	assert.equal(sdk.expandPath("file\u00a0name.txt"), "file name.txt");
	const outside = path.join(owned, "outside");
	fs.mkdirSync(outside);
	fs.symlinkSync(outside, path.join(cwd, "outside"));
	assert.throws(
		() => sdk.resolveContainedPath("outside/secret.txt", cwd),
		/outside the workspace|symlinked path/i,
	);
	fs.writeFileSync(path.join(cwd, "Cyber_harness.md"), "# spec\n");
	assert.throws(
		() =>
			sdk.resolveContainedPath(
				"/home/runner/work/cyber-h/advanced-cyber-harness/Cyber_harness.md",
				cwd,
			),
		(error) =>
			error.message.includes("Path escapes the workspace") &&
			error.message.includes("Workspace root: " + cwd) &&
			error.message.includes("Cyber_harness.md"),
	);
} finally {
	fs.rmSync(owned, { recursive: true, force: true });
}
const out = {
	version: 1,
	source_commit: "e30f0b4b238ee5618928bcd17b4571eb046e7038",
	rows,
	lexical_assertions: 5,
	repair_assertions: 2,
	escape_assertions: 2,
};
const target = path.join(root, "fixtures/tools/path-oracle.json");
if (process.argv.includes("--capture")) {
	assert(!fs.existsSync(target));
	fs.writeFileSync(target, JSON.stringify(out, null, 2) + "\n");
} else assert.deepEqual(out, JSON.parse(fs.readFileSync(target, "utf8")));
console.log(
	JSON.stringify({
		status: "passed",
		read_cases: rows.length,
		lexical_assertions: 5,
		repair_assertions: 2,
		escape_assertions: 2,
		model_actions: 0,
		network_actions: 0,
	}),
);
