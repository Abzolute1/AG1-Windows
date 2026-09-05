// Immutable SDK credential-change refresh oracle; owned config and fixture auth only.
import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { fileURLToPath, pathToFileURL } from "node:url";
await import("../offline_guard.mjs");
const root = path.resolve(
	path.dirname(fileURLToPath(import.meta.url)),
	"../..",
);
const { ModelRegistry } = await import(
	pathToFileURL(
		path.join(
			root,
			"reference/ag1-executable/packages/coding-agent/dist/core/model-registry.js",
		),
	)
);
const owned = fs.mkdtempSync(path.join(root, ".local/p09-provider-refresh-"));
try {
	const configPath = path.join(owned, "models.json");
	fs.writeFileSync(configPath, JSON.stringify({ providers: {} }));
	let credential;
	const registry = ModelRegistry.create(
		{ getOAuthProviders: () => [], get: () => credential },
		configPath,
	);
	const config = {
		api: "openai-completions",
		baseUrl: "https://fixture.invalid/v1",
		models: [
			{
				id: "model",
				name: "Model",
				reasoning: false,
				input: ["text"],
				cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
				contextWindow: 8192,
				maxTokens: 1024,
			},
		],
		oauth: { name: "Fixture" },
		hooks: { oauth: { modifyModels: true } },
	};
	let calls = 0;
	const nativeConfig = structuredClone(config);
	delete nativeConfig.hooks;
	nativeConfig.oauth.modifyModels = (models, c) => {
		calls++;
		return models.map((m) =>
			m.provider === "fixture"
				? { ...m, name: m.name + " for " + c.projectId }
				: m,
		);
	};
	registry.registerProvider("fixture", nativeConfig);
	const rows = [];
	const capture = (action) =>
		rows.push({
			action,
			models: registry.getAll().filter((m) => m.provider === "fixture"),
			calls,
		});
	capture("register");
	credential = {
		type: "oauth",
		access: "fixture-private-access",
		refresh: "fixture-private-refresh",
		expires: 1000000,
		projectId: "fixture-account",
	};
	registry.refresh();
	capture("login");
	registry.refresh();
	capture("repeat");
	credential = undefined;
	registry.refresh();
	capture("logout");
	const result = {
		version: 1,
		source_commit: "e30f0b4b238ee5618928bcd17b4571eb046e7038",
		config,
		rows,
	};
	const target = path.join(root, "fixtures/tools/provider-refresh-oracle.json");
	if (process.argv.includes("--capture")) {
		assert(!fs.existsSync(target));
		fs.writeFileSync(target, JSON.stringify(result, null, 2) + "\n");
	} else
		assert.deepEqual(
			JSON.parse(JSON.stringify(result)),
			JSON.parse(fs.readFileSync(target, "utf8")),
		);
	console.log(
		JSON.stringify({
			status: "passed",
			credential_change_refresh_states: rows.length,
			external_actions: 0,
		}),
	);
} finally {
	fs.rmSync(owned, { recursive: true, force: true });
}
