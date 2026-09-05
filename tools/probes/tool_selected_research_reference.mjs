// Original selected research assertions; data functions use the immutable SDK.
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
const sourcePath = "packages/coding-agent/test/web-tools-context.test.ts";
const filename = path.join(root, "reference/ag1-executable", sourcePath);
const source = fs.readFileSync(filename, "utf8");
const base = path.join(
	root,
	"reference/ag1-executable/packages/coding-agent/dist/core/tools",
);
const tests = [],
	rows = [],
	after = [];
const searchMode = process.argv.includes("--search");
const renderMode = process.argv.includes("--render");
const before = [];
const RealDate = Date;
if (searchMode)
	globalThis.Date = class extends RealDate {
		constructor(...args) {
			super(...(args.length ? args : ["2026-09-05T00:00:00.000Z"]));
		}
		static now() {
			return RealDate.parse("2026-09-05T00:00:00.000Z");
		}
	};
const owned = fs.mkdtempSync(path.join(root, ".local/p09-selected-research-"));
const globalValues = new Map(),
	envValues = new Map();
let activeSearch;
let current,
	assertions = 0;
function subset(actual, expected) {
	for (const [key, value] of Object.entries(expected)) {
		if (value && typeof value === "object") subset(actual[key], value);
		else assert.deepEqual(actual[key], value);
	}
}
function expect(actual) {
	const match = (negative) => ({
		toBe(expected) {
			assertions++;
			assert.equal(Object.is(actual, expected), !negative);
		},
		toEqual(expected) {
			assertions++;
			assert(!negative);
			if (expected?.arrayContaining)
				for (const item of expected.arrayContaining)
					assert(
						actual.some((value) => {
							if (item?.stringContaining !== undefined)
								return (
									typeof value === "string" &&
									value.includes(item.stringContaining)
								);
							try {
								assert.deepEqual(value, item);
								return true;
							} catch {
								return false;
							}
						}),
					);
			// Vitest toEqual ignores object properties whose value is undefined.
			else
				assert.deepEqual(
					JSON.parse(JSON.stringify(actual)),
					JSON.parse(JSON.stringify(expected)),
				);
		},
		toContain(expected) {
			assertions++;
			assert.equal(actual.includes(expected), !negative);
		},
		toMatch(pattern) {
			assertions++;
			assert.equal(pattern.test(actual), !negative);
		},
		toHaveLength(expected) {
			assertions++;
			assert(!negative);
			assert.equal(actual.length, expected);
		},
		toMatchObject(expected) {
			assertions++;
			assert(!negative);
			subset(actual, expected);
		},
		toHaveBeenCalledTimes(expected) {
			assertions++;
			assert(!negative);
			assert.equal(actual.mock.calls.length, expected);
		},
		toHaveBeenCalled() {
			assertions++;
			assert.equal(actual.mock.calls.length > 0, !negative);
		},
		toBeLessThanOrEqual(expected) {
			assertions++;
			assert(!negative);
			assert(actual <= expected);
		},
		toBeLessThan(expected) {
			assertions++;
			assert(!negative);
			assert(actual < expected);
		},
		toBeGreaterThan(expected) {
			assertions++;
			assert(!negative);
			assert(actual > expected);
		},
		get not() {
			return match(!negative);
		},
	});
	return match(false);
}
expect.arrayContaining = (values) => ({ arrayContaining: values });
expect.stringContaining = (value) => ({ stringContaining: value });
const suite = {
	describe(_name, fn) {
		fn();
	},
	beforeAll(fn) {
		before.push(fn);
	},
	afterEach(fn) {
		after.push(fn);
	},
	test(name, run) {
		tests.push({ name: "web research tool context efficiency " + name, run });
	},
	expect,
	vi: {
		fn(implementation = () => undefined) {
			const once = [];
			const mock = (...args) => {
				mock.mock.calls.push(args);
				return once.length
					? Promise.resolve(once.shift())
					: implementation(...args);
			};
			mock.mock = { calls: [] };
			mock.mockResolvedValueOnce = (value) => {
				once.push(value);
				return mock;
			};
			return mock;
		},
		stubGlobal(key, value) {
			assert.equal(key, "fetch");
			if (!globalValues.has(key)) globalValues.set(key, globalThis[key]);
			globalThis[key] = async (url, options) => {
				assert(activeSearch);
				const trace = { url: String(url), headers: options?.headers };
				activeSearch.requests.push(trace);
				const response = await value(url, options);
				trace.ok = response.ok;
				const wrapped = { ...response };
				for (const method of ["text", "json"])
					if (response[method])
						wrapped[method] = async () => {
							const body = await response[method]();
							trace[method] = body;
							return body;
						};
				return wrapped;
			};
		},
		stubEnv(key, value) {
			assert(
				[
					"AG1_BRAVE_SEARCH_API_KEY",
					"AG1_WEB_SEARCH_PROVIDER",
					"AG1_CODING_AGENT_DIR",
				].includes(key),
			);
			if (!envValues.has(key)) envValues.set(key, process.env[key]);
			process.env[key] = value;
		},
		unstubAllGlobals() {
			for (const [key, value] of globalValues) globalThis[key] = value;
			globalValues.clear();
		},
		unstubAllEnvs() {
			for (const [key, value] of envValues) {
				if (value === undefined) delete process.env[key];
				else process.env[key] = value;
			}
			envValues.clear();
		},
		restoreAllMocks() {},
	},
};
const fail = () => {
	throw Error("Unselected network or UI API");
};
const modules = {
	"fixture:vitest": suite,
	"fixture:index": {
		createWebSearchTool: fail,
		webFetchToolDefinition: {},
		webSearchToolDefinition: {},
	},
	"fixture:component": { ToolExecutionComponent: fail },
	"fixture:theme": { initTheme: fail },
	"fixture:ansi": { default: fail },
};
for (const name of ["research-intent", "research-output", "websearch"]) {
	const sdk = await import(pathToFileURL(path.join(base, name + ".js")));
	modules["fixture:" + name] = Object.fromEntries(
		Object.entries(sdk).map(([key, fn]) => [
			key,
			typeof fn !== "function"
				? fn
				: (...args) => {
						const result = fn(...args);
						if (current && key === "rememberResearchFetchOutcome")
							current.outcomes[args[0]] = args[1];
						if (
							current &&
							!searchMode &&
							!/^(remember|reset|getResearchUrlHint)/.test(key)
						)
							current.calls.push({ name: key, args, result: result ?? null });
						return result;
					},
		]),
	);
}
if (searchMode) {
	const sdk = await import(pathToFileURL(path.join(base, "websearch.js")));
	const capture = (definition) => ({
		...definition,
		async execute(id, args, signal, progress, context) {
			activeSearch = {
				args,
				usage: context?.getContextUsage?.() ?? {},
				requests: [],
				outcomes: structuredClone(current.outcomes),
			};
			try {
				activeSearch.output = await definition.execute(
					id,
					args,
					signal,
					progress,
					context,
				);
				current.calls.push(activeSearch);
				return activeSearch.output;
			} finally {
				activeSearch = undefined;
			}
		},
	});
	modules["fixture:index"] = {
		createWebSearchTool: (options) =>
			capture(sdk.createWebSearchTool({ ...options, cwd: owned })),
		webSearchToolDefinition: capture(sdk.createWebSearchTool({ cwd: owned })),
		webFetchToolDefinition: {},
	};
}
if (renderMode) {
	suite.vi.stubEnv("AG1_CODING_AGENT_DIR", owned);
	const dist = path.join(
		root,
		"reference/ag1-executable/packages/coding-agent/dist",
	);
	const { ToolExecutionComponent } = await import(
		pathToFileURL(
			path.join(dist, "modes/interactive/components/tool-execution.js"),
		)
	);
	const theme = await import(
		pathToFileURL(path.join(dist, "modes/interactive/theme/theme.js"))
	);
	const ansi = await import(
		pathToFileURL(
			path.join(
				root,
				"reference/ag1-executable/node_modules/strip-ansi/index.js",
			),
		)
	);
	const search = await import(pathToFileURL(path.join(base, "websearch.js")));
	const fetch = await import(pathToFileURL(path.join(base, "webfetch.js")));
	modules["fixture:index"] = {
		createWebSearchTool: fail,
		webSearchToolDefinition: search.createWebSearchTool({ cwd: owned }),
		webFetchToolDefinition: fetch.createWebFetchTool({ cwd: owned }),
	};
	modules["fixture:theme"] = theme;
	modules["fixture:ansi"] = ansi;
	modules["fixture:component"] = {
		ToolExecutionComponent: class extends ToolExecutionComponent {
			constructor(...args) {
				super(...args);
				this.fixtureTool = args[0];
				this.fixtureArgs = args[2];
				this.fixtureExpanded = false;
			}
			updateResult(result, ...args) {
				this.fixtureResult = result;
				return super.updateResult(result, ...args);
			}
			setExpanded(expanded) {
				this.fixtureExpanded = expanded;
				return super.setExpanded(expanded);
			}
			render(width) {
				const rendered = super.render(width);
				const search = this.fixtureTool === "webSearch";
				current.calls.push({
					tool: this.fixtureTool,
					args: this.fixtureArgs,
					result: this.fixtureResult,
					expanded: this.fixtureExpanded,
					presentation: {
						heading: this.fixtureTool.toUpperCase(),
						input: this.fixtureArgs[search ? "query" : "url"],
						summary: search
							? this.summarizeWebSearchResult()
							: this.summarizeWebFetchResult(),
						body: search
							? this.formatCompactWebSearchResult()
							: this.formatCompactWebFetchResult(),
					},
					rendered: ansi.default(rendered.join("\n")),
					width,
				});
				return rendered;
			}
		},
	};
}
const jiti = createJiti(import.meta.url, {
	moduleCache: false,
	fsCache: false,
	tryNative: false,
	virtualModules: modules,
});
let adapted = source;
for (const [from, to] of Object.entries({
	vitest: "vitest",
	"strip-ansi": "ansi",
	"../src/core/tools/index.js": "index",
	"../src/core/tools/research-intent.js": "research-intent",
	"../src/core/tools/research-output.js": "research-output",
	"../src/core/tools/websearch.js": "websearch",
	"../src/modes/interactive/components/tool-execution.js": "component",
	"../src/modes/interactive/theme/theme.js": "theme",
}))
	adapted = adapted.replace(`from "${from}"`, `from "fixture:${to}"`);
await jiti.evalModule(adapted, { filename, async: true });
const selected = tests.filter((t) =>
	renderMode
		? t.name.includes(" renders ")
		: searchMode
			? t.run.constructor.name === "AsyncFunction" &&
				!t.run.toString().includes("webFetchToolDefinition")
			: t.run.constructor.name !== "AsyncFunction" &&
				!t.name.includes(" renders "),
);
assert.equal(selected.length, renderMode ? 4 : searchMode ? 32 : 27);
try {
	if (renderMode) for (const fn of before) await fn();
	for (const test of selected) {
		current = { name: test.name, calls: [] };
		if (searchMode) {
			current.outcomes = {};
			suite.vi.stubEnv("AG1_BRAVE_SEARCH_API_KEY", "");
			suite.vi.stubEnv("AG1_WEB_SEARCH_PROVIDER", "auto");
		}
		const start = assertions;
		try {
			await test.run();
			current.assertions = assertions - start;
			assert(current.calls.length && current.assertions);
			rows.push(current);
		} finally {
			current = undefined;
			for (const fn of after) await fn();
		}
	}
} finally {
	globalThis.Date = RealDate;
	suite.vi.unstubAllGlobals();
	suite.vi.unstubAllEnvs();
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
	renderMode
		? "fixtures/tools/selected-web-presentation-oracle.json"
		: searchMode
			? "fixtures/tools/selected-websearch-oracle.json"
			: "fixtures/tools/selected-research-oracle.json",
);
if (process.argv.includes("--capture")) {
	assert(!fs.existsSync(target));
	fs.writeFileSync(target, JSON.stringify(result, null, 2) + "\n");
} else assert.deepEqual(result, JSON.parse(fs.readFileSync(target, "utf8")));
console.log(
	JSON.stringify({
		status: "passed",
		selected_research_cases: rows.length,
		original_assertions: assertions,
		network_actions: 0,
		renderer_cases: renderMode ? rows.length : 0,
	}),
);
