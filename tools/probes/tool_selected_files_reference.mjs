// Run selected immutable source assertions and capture actual SDK calls for native replay.
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
const sourcePath = "packages/coding-agent/test/tools.test.ts";
const filename = path.join(root, "reference/ag1-executable", sourcePath);
const source = fs.readFileSync(filename, "utf8");
const owned = fs.mkdtempSync(path.join(root, ".local/p09-selected-files-"));
const searchMode = process.argv.includes("--search");
if (searchMode) {
	process.env.AG1_CODING_AGENT_DIR = path.join(owned, "agent");
	process.env.PATH = "/usr/bin:/bin";
	process.env.RIPGREP_CONFIG_PATH = "/dev/null";
}
function tree(cwd) {
	const result = {};
	function walk(dir) {
		for (const entry of fs
			.readdirSync(dir, { withFileTypes: true })
			.sort((a, b) => a.name.localeCompare(b.name))) {
			const full = path.join(dir, entry.name),
				rel = path.relative(cwd, full);
			assert(!entry.isSymbolicLink());
			if (entry.isDirectory()) {
				result[rel] = { directory: true };
				walk(full);
			} else {
				result[rel] = {
					bytes: fs.readFileSync(full).toString("base64"),
					mode: fs.statSync(full).mode & 0o777,
				};
			}
			assert(Object.keys(result).length <= 2048);
		}
	}
	walk(cwd);
	return result;
}
const suites = [],
	tests = [],
	rows = [];
let current,
	assertions = 0;
function subset(actual, expected) {
	if (expected && typeof expected === "object") {
		assert(actual && typeof actual === "object");
		for (const [key, value] of Object.entries(expected))
			subset(actual[key], value);
	} else assert.deepEqual(actual, expected);
}
function expect(actual) {
	const match = (negated) => {
		const check = (ok) => {
			assertions++;
			assert.equal(!!ok, !negated);
		};
		return {
			toBe: (expected) => check(Object.is(actual, expected)),
			toBeDefined: () => check(actual !== undefined),
			toBeUndefined: () => check(actual === undefined),
			toBeTruthy: () => check(!!actual),
			toContain: (expected) => check(actual?.includes(expected)),
			toMatch: (expected) => check(expected.test(actual)),
			toBeLessThan: (expected) => check(actual < expected),
			toBeGreaterThan: (expected) => check(actual > expected),
			toBeGreaterThanOrEqual: (expected) => check(actual >= expected),
			toEqual: (expected) => {
				assertions++;
				(negated ? assert.notDeepEqual : assert.deepEqual)(actual, expected);
			},
			toMatchObject: (expected) => {
				assertions++;
				assert(!negated);
				subset(actual, expected);
			},
			get not() {
				return match(!negated);
			},
			get rejects() {
				return {
					async toThrow(pattern) {
						assertions++;
						await assert.rejects(actual, pattern);
					},
				};
			},
		};
	};
	return match(false);
}
const suite = {
	describe(name, fn) {
		suites.push({ name, before: [], after: [] });
		fn();
		suites.pop();
	},
	beforeEach(fn) {
		suites.at(-1).before.push(fn);
	},
	afterEach(fn) {
		suites.at(-1).after.push(fn);
	},
	it(name, run) {
		tests.push({
			name: [...suites.map((s) => s.name), name].join(" "),
			run,
			before: suites.flatMap((s) => s.before),
			after: suites.toReversed().flatMap((s) => s.after),
		});
	},
	expect,
	vi: new Proxy(
		{},
		{
			get() {
				throw Error("unselected mock API");
			},
		},
	),
};
const modules = {
	"fixture:vitest": suite,
	"fixture:os": { tmpdir: () => owned },
};
const stub = () => ({});
for (const tool of [
	"read",
	"write",
	"edit",
	...(searchMode ? ["find", "grep", "ls"] : []),
]) {
	const sdk = await import(
		pathToFileURL(
			path.join(
				root,
				"reference/ag1-executable/packages/coding-agent/dist/core/tools",
				tool + ".js",
			),
		)
	);
	const key = "create" + tool[0].toUpperCase() + tool.slice(1) + "Tool";
	modules["fixture:" + tool] = {
		[key]: (cwd, ...options) => {
			assert(cwd.startsWith(owned + path.sep));
			const definition = sdk[key](cwd, ...options);
			return {
				...definition,
				async execute(id, args, ...rest) {
					const target = path.resolve(cwd, args.path ?? ".");
					assert(searchMode || target.startsWith(cwd + path.sep));
					const state = () => {
						if (searchMode) return tree(cwd);
						if (!fs.existsSync(target)) return null;
						const stat = fs.statSync(target);
						return stat.isDirectory()
							? { directory: true }
							: {
									bytes: fs.readFileSync(target).toString("base64"),
									mode: stat.mode & 0o777,
								};
					};
					const normalize = (value) =>
						JSON.parse(JSON.stringify(value).replaceAll(cwd, "/fixture"));
					const row = {
						tool,
						...(searchMode ? { callId: id } : {}),
						args: normalize(args),
						before: state(),
					};
					try {
						const result = await definition.execute(id, args, ...rest);
						row.output = normalize({ result });
						return result;
					} catch (error) {
						row.output = normalize({ error: error.message });
						throw error;
					} finally {
						row.after = state();
						current.calls.push(row);
					}
				},
			};
		},
	};
}
for (const tool of searchMode ? ["bash"] : ["bash", "find", "grep", "ls"])
	modules["fixture:" + tool] = {
		["create" + tool[0].toUpperCase() + tool.slice(1) + "Tool"]: stub,
		createLocalBashOperations: stub,
	};
modules["fixture:executor"] = {
	executeBash: () => {
		throw Error("unselected shell execution");
	},
};
modules["fixture:shell"] = {};
const jiti = createJiti(import.meta.url, {
	moduleCache: false,
	fsCache: false,
	tryNative: false,
	virtualModules: modules,
});
try {
	let adapted = source
		.replace('from "vitest"', 'from "fixture:vitest"')
		.replace('from "os"', 'from "fixture:os"')
		.replace('from "../src/core/bash-executor.js"', 'from "fixture:executor"')
		.replace('from "../src/utils/shell.js"', 'from "fixture:shell"');
	for (const tool of ["read", "write", "edit", "bash", "find", "grep", "ls"])
		adapted = adapted.replace(
			`from "../src/core/tools/${tool}.js"`,
			`from "fixture:${tool}"`,
		);
	await jiti.evalModule(adapted, { filename, async: true });
	const selected = tests.filter((t) =>
		searchMode
			? /^Coding Agent Tools ((grep|find|ls) tool |structured local metadata )/.test(
					t.name,
				)
			: /^(Coding Agent Tools (read|write|edit) tool |edit tool (fuzzy matching|CRLF handling) )/.test(
					t.name,
				) && !t.name.includes("explicitly loaded skill root"),
	);
	assert.equal(selected.length, searchMode ? 27 : 42);
	for (const test of selected) {
		current = { name: test.name, calls: [] };
		const start = assertions;
		try {
			for (const fn of test.before) await fn();
			await test.run();
			current.assertions = assertions - start;
			assert(current.calls.length);
			rows.push(current);
		} finally {
			for (const fn of test.after) await fn();
		}
	}
} finally {
	fs.rmSync(owned, { recursive: true, force: true });
}
const result = {
	version: 1,
	source_commit: "e30f0b4b238ee5618928bcd17b4571eb046e7038",
	source_test: {
		path: sourcePath,
		sha256: createHash("sha256").update(source).digest("hex"),
	},
	rows,
};
const target = path.join(
	root,
	searchMode
		? "fixtures/tools/selected-search-oracle.json"
		: "fixtures/tools/selected-files-oracle.json",
);
if (process.argv.includes("--capture")) {
	assert(!fs.existsSync(target));
	fs.writeFileSync(target, JSON.stringify(result, null, 2) + "\n");
} else assert.deepEqual(result, JSON.parse(fs.readFileSync(target, "utf8")));
console.log(
	JSON.stringify({
		status: "passed",
		selected_file_cases: rows.length,
		source_assertions: assertions,
		external_actions: 0,
	}),
);
