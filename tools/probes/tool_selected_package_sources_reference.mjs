// Execute the immutable source's parsing/identity assertions with the real SDK.
import fs from "node:fs";
import path from "node:path";
import os from "node:os";
import { syncBuiltinESMExports } from "node:module";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { fileURLToPath, pathToFileURL } from "node:url";
import { createJiti } from "../../product/compat/extensions/vendor/node_modules/@mariozechner/jiti/lib/jiti.mjs";
await import("../offline_guard.mjs");
const root = path.resolve(
	path.dirname(fileURLToPath(import.meta.url)),
	"../..",
);
const sourcePath = "packages/coding-agent/test/package-manager.test.ts";
const filename = path.join(root, "reference/ag1-executable", sourcePath);
const source = fs.readFileSync(filename, "utf8");
const base = path.join(
	root,
	"reference/ag1-executable/packages/coding-agent/dist",
);
const { DefaultPackageManager } = await import(
	pathToFileURL(path.join(base, "core/package-manager.js"))
);
const settings = await import(
	pathToFileURL(path.join(base, "core/settings-manager.js"))
);
const config = await import(pathToFileURL(path.join(base, "config.js")));
const owned = fs.mkdtempSync(
	path.join(root, ".local/p09-selected-package-sources-"),
);
const explicitMode = process.argv.includes("--explicit");
const resourceMode = process.argv.includes("--resources") || explicitMode;
const exists = fs.existsSync,
	homedir = os.homedir;
let fixtureRoot;
if (resourceMode) {
	os.homedir = () => path.join(fixtureRoot ?? owned, "home");
	fs.existsSync = (input) => {
		const value = String(input);
		if (
			!value.startsWith(owned + path.sep) &&
			value.includes("/.agents/skills")
		)
			return false;
		return exists(input);
	};
	syncBuiltinESMExports();
}
function tree(dir) {
	const files = {};
	const walk = (at) => {
		for (const entry of fs
			.readdirSync(at, { withFileTypes: true })
			.sort((a, b) => a.name.localeCompare(b.name))) {
			assert(!entry.isSymbolicLink());
			const full = path.join(at, entry.name);
			if (entry.isDirectory()) {
				files[path.relative(dir, full)] = { directory: true };
				walk(full);
			} else
				files[path.relative(dir, full)] = {
					text: fs.readFileSync(full, "utf8"),
				};
			assert(Object.keys(files).length < 2048);
		}
	};
	walk(dir);
	return files;
}
const suites = [],
	tests = [],
	rows = [];
let current,
	assertions = 0;
class CapturedManager extends DefaultPackageManager {
	constructor(options) {
		assert(options.cwd.startsWith(owned + path.sep));
		super(options);
		this.fixtureOptions = options;
		fixtureRoot = path.join(
			owned,
			path.relative(owned, options.cwd).split(path.sep)[0],
		);
	}
	parseSource(source) {
		const output = super.parseSource(source);
		if (!resourceMode) current.calls.push({ method: "parse", source, output });
		return output;
	}
	getPackageIdentity(source, scope) {
		const output = super.getPackageIdentity(source, scope);
		if (!resourceMode)
			current.calls.push({ method: "identity", source, output });
		return output;
	}
	async resolve() {
		assert(resourceMode);
		const { cwd, agentDir, settingsManager } = this.fixtureOptions;
		const input = {
			cwd,
			agentDir,
			home: os.homedir(),
			files: tree(fixtureRoot),
			user: settingsManager.getGlobalSettings(),
			project: settingsManager.getProjectSettings(),
		};
		const output = await super.resolve();
		current.calls.push(
			JSON.parse(
				JSON.stringify({ input, output }).replaceAll(fixtureRoot, "/fixture"),
			),
		);
		return output;
	}
	async resolveExtensionSources(sources, options) {
		assert(explicitMode);
		const { cwd, agentDir, settingsManager } = this.fixtureOptions;
		const input = {
			cwd,
			agentDir,
			home: os.homedir(),
			files: tree(fixtureRoot),
			sources,
			options: options ?? {},
			user: settingsManager.getGlobalSettings(),
			project: settingsManager.getProjectSettings(),
		};
		const output = await super.resolveExtensionSources(sources, options);
		current.calls.push(
			JSON.parse(
				JSON.stringify({ input, output }).replaceAll(fixtureRoot, "/fixture"),
			),
		);
		return output;
	}
	runCommand() {
		throw Error("Unselected package command");
	}
	runCommandSync() {
		throw Error("Unselected package command");
	}
	runCommandCapture() {
		throw Error("Unselected package command");
	}
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
	expect(actual) {
		const match = (negated) => ({
			toEqual(expected) {
				assertions++;
				assert(!negated);
				assert.deepEqual(actual, expected);
			},
			toHaveLength(expected) {
				assertions++;
				assert(!negated);
				assert.equal(actual.length, expected);
			},
			toBe(expected) {
				assertions++;
				negated
					? assert.notStrictEqual(actual, expected)
					: assert.strictEqual(actual, expected);
			},
			get not() {
				return match(!negated);
			},
		});
		return match(false);
	},
	vi: { restoreAllMocks() {}, unstubAllGlobals() {} },
};
const modules = {
	"fixture:vitest": suite,
	"fixture:os": { tmpdir: () => owned },
	"fixture:manager": { DefaultPackageManager: CapturedManager },
	"fixture:settings": settings,
	"fixture:config": config,
};
const jiti = createJiti(import.meta.url, {
	moduleCache: false,
	fsCache: false,
	tryNative: false,
	virtualModules: modules,
});
try {
	let adapted = source;
	for (const [from, to] of Object.entries({
		vitest: "vitest",
		"node:os": "os",
		"../src/core/package-manager.js": "manager",
		"../src/core/settings-manager.js": "settings",
		"../src/config.js": "config",
	})) {
		adapted = adapted.replace(`from "${from}"`, `from "fixture:${to}"`);
	}
	await jiti.evalModule(adapted, { filename, async: true });
	const sourceSelected = tests.filter(
		(t) =>
			t.name.includes("HTTPS git URL parsing (old behavior)") ||
			/source parsing should (parse package source types|never parse dot-relative)/.test(
				t.name,
			) ||
			/package deduplication should (dedupe (SSH|all supported)|keep different repos)/.test(
				t.name,
			),
	);
	const selected = explicitMode
		? tests.filter(
				(t) =>
					t.run.toString().includes("resolveExtensionSources") &&
					!t.name.includes("offline mode"),
			)
		: resourceMode
			? tests.filter(
					(t) =>
						/^DefaultPackageManager (resolve |\.agents\/skills auto-discovery |ignore files |pattern filtering in (top-level arrays|package filters) |force-(include|exclude) patterns |multi-file extension discovery |package deduplication should (dedupe same local|keep both))/.test(
							t.name,
						) &&
						!t.name.includes("keep ~/.agents/skills") &&
						!t.run.toString().includes("resolveExtensionSources"),
				)
			: sourceSelected;
	assert.equal(selected.length, explicitMode ? 12 : resourceMode ? 33 : 19);
	for (const test of selected) {
		current = { name: test.name, calls: [] };
		const start = assertions;
		try {
			for (const fn of test.before) await fn();
			await test.run();
			current.assertions = assertions - start;
			assert(current.calls.length && current.assertions);
			rows.push(current);
		} finally {
			for (const fn of test.after) await fn();
		}
	}
} finally {
	fs.existsSync = exists;
	os.homedir = homedir;
	syncBuiltinESMExports();
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
		rows,
	}),
);
const target = path.join(
	root,
	explicitMode
		? "fixtures/tools/selected-package-explicit-oracle.json"
		: resourceMode
			? "fixtures/tools/selected-package-resources-oracle.json"
			: "fixtures/tools/selected-package-sources-oracle.json",
);
if (process.argv.includes("--capture")) {
	assert(!fs.existsSync(target));
	fs.writeFileSync(target, JSON.stringify(result, null, 2) + "\n");
} else assert.deepEqual(result, JSON.parse(fs.readFileSync(target, "utf8")));
console.log(
	JSON.stringify({
		status: "passed",
		selected_package_cases: rows.length,
		original_assertions: assertions,
		network_actions: 0,
		external_commands: 0,
	}),
);
