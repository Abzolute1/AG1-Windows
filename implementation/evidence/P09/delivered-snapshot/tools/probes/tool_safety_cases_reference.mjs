// Execute the immutable selected policy tests with their original assertions;
// capture each policy call for native replay. Shell strings are never executed.
import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { fileURLToPath, pathToFileURL } from "node:url";
import { createJiti } from "../../product/compat/extensions/vendor/node_modules/@mariozechner/jiti/lib/jiti.mjs";
await import("../offline_guard.mjs");
const root = path.resolve(
	path.dirname(fileURLToPath(import.meta.url)),
	"../..",
);
const sourcePath = "packages/coding-agent/test/safety-policy.test.ts";
const filename = path.join(root, "reference/ag1-executable", sourcePath);
const source = fs.readFileSync(filename, "utf8");
const sdk = await import(
	pathToFileURL(
		path.join(
			root,
			"reference/ag1-executable/packages/coding-agent/dist/core/safety-policy.js",
		),
	)
);
const owned = fs.mkdtempSync(path.join(root, ".local/p09-safety-cases-"));
const tests = [],
	before = [],
	after = [],
	rows = [];
let current,
	aliases = [],
	symlinks = [],
	assertions = 0;
const normalize = (value) => {
	if (typeof value === "string") {
		for (const [from, to] of aliases) value = value.replaceAll(from, to);
		return value;
	}
	if (Array.isArray(value)) return value.map(normalize);
	if (value && typeof value === "object")
		return Object.fromEntries(
			Object.entries(value).map(([k, v]) => [k, normalize(v)]),
		);
	return value;
};
function expect(actual, label) {
	function match(negated) {
		const check = (ok) => {
			assertions++;
			assert.equal(!!ok, !negated, label);
		};
		return {
			toBe: (expected) => check(Object.is(actual, expected)),
			toBeUndefined: () => check(actual === undefined),
			toContain: (expected) => check(actual?.includes(expected)),
			toMatch: (expected) => check(expected.test(actual)),
			toEqual: (expected) => {
				assertions++;
				if (negated) assert.notDeepEqual(actual, expected, label);
				else assert.deepEqual(actual, expected, label);
			},
			get not() {
				return match(!negated);
			},
		};
	}
	return match(false);
}
const suite = {
	describe: (name, fn) => {
		assert.equal(name, "tool safety policy");
		fn();
	},
	it: (name, run) => tests.push({ name, run }),
	beforeEach: (fn) => before.push(fn),
	afterEach: (fn) => after.push(fn),
	expect,
};
const filesystem = {
	...fs,
	mkdtempSync(prefix) {
		assert(prefix.startsWith(owned + path.sep));
		const dir = fs.mkdtempSync(prefix);
		aliases.push([dir, prefix.includes("outside") ? "/outside" : "/fixture"]);
		return dir;
	},
	symlinkSync(target, link, ...args) {
		assert(target.startsWith(owned + path.sep));
		assert(link.startsWith(owned + path.sep));
		symlinks.push({ path: normalize(link), target: normalize(target) });
		return fs.symlinkSync(target, link, ...args);
	},
};
const policy = {
	...sdk,
	evaluateToolSafety(tool, args, options) {
		const result = sdk.evaluateToolSafety(tool, args, options);
		current.calls.push(
			normalize({
				input: {
					tool,
					args,
					policy: options,
					untrusted: process.env.AG1_AUTONOMOUS_UNTRUSTED_EXTENSIONS === "1",
				},
				output: result,
				dialog: result.approvalRequest
					? sdk.formatToolApprovalDialog(result.approvalRequest)
					: null,
			}),
		);
		return result;
	},
};
const jiti = createJiti(import.meta.url, {
	moduleCache: false,
	fsCache: false,
	tryNative: false,
	virtualModules: {
		"fixture:vitest": suite,
		"fixture:policy": policy,
		"fixture:fs": filesystem,
		"fixture:os": { tmpdir: () => owned },
	},
});
const originalUntrusted = process.env.AG1_AUTONOMOUS_UNTRUSTED_EXTENSIONS;
delete process.env.AG1_AUTONOMOUS_UNTRUSTED_EXTENSIONS;
try {
	const adapted = source
		.replace('from "vitest"', 'from "fixture:vitest"')
		.replace('from "../src/core/safety-policy.js"', 'from "fixture:policy"')
		.replace('from "node:fs"', 'from "fixture:fs"')
		.replace('from "node:os"', 'from "fixture:os"');
	await jiti.evalModule(adapted, { filename, async: true });
	assert.equal(tests.length, 50);
	for (const test of tests) {
		current = { name: `tool safety policy ${test.name}`, calls: [] };
		aliases = [];
		symlinks = [];
		const start = assertions;
		try {
			for (const setup of before) await setup();
			await test.run();
			current.assertions = assertions - start;
			current.symlinks = symlinks;
			assert(current.calls.length > 0);
			rows.push(current);
		} finally {
			for (const teardown of after) await teardown();
		}
	}
} finally {
	if (originalUntrusted === undefined)
		delete process.env.AG1_AUTONOMOUS_UNTRUSTED_EXTENSIONS;
	else process.env.AG1_AUTONOMOUS_UNTRUSTED_EXTENSIONS = originalUntrusted;
	fs.rmSync(owned, { recursive: true, force: true });
}
const result = JSON.parse(
	JSON.stringify({
		version: 1,
		source_commit: "e30f0b4b238ee5618928bcd17b4571eb046e7038",
		source_test: {
			path: sourcePath,
			sha256: createHash("sha256").update(source).digest("hex"),
		},
		model_actions: 0,
		network_actions: 0,
		rows,
	}),
);
const target = path.join(root, "fixtures/tools/safety-cases-oracle.json");
if (process.argv.includes("--capture")) {
	assert(!fs.existsSync(target));
	fs.writeFileSync(target, JSON.stringify(result, null, 2) + "\n");
} else assert.deepEqual(result, JSON.parse(fs.readFileSync(target, "utf8")));
console.log(
	JSON.stringify({
		status: "passed",
		selected_safety_cases: rows.length,
		policy_calls: rows.reduce((n, row) => n + row.calls.length, 0),
		source_assertions: assertions,
		model_actions: 0,
		network_actions: 0,
	}),
);
