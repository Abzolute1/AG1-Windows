import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { syncBuiltinESMExports } from "node:module";
import { fileURLToPath, pathToFileURL } from "node:url";
await import("../offline_guard.mjs");
const root = path.resolve(
	path.dirname(fileURLToPath(import.meta.url)),
	"../..",
);
const owned = fs.mkdtempSync(path.join(root, ".local/p09-loader-selection-"));
const exists = fs.existsSync;
fs.existsSync = function (input) {
	const p = String(input);
	if (
		!p.startsWith(owned + path.sep) &&
		(p.endsWith("/AGENTS.md") ||
			p.endsWith("/CLAUDE.md") ||
			p.includes("/.agents/skills"))
	)
		return false;
	return exists(input);
};
syncBuiltinESMExports();
const { DefaultResourceLoader } = await import(
	pathToFileURL(
		path.join(
			root,
			"reference/ag1-executable/packages/coding-agent/dist/core/resource-loader.js",
		),
	)
);
const theme = JSON.parse(
	fs.readFileSync(path.join(root, "fixtures/tools/theme-oracle.json"), "utf8"),
).rows[0].input;
const skill = (name) =>
	`---\nname: ${name}\ndescription: ${name} skill.\n---\nBody.`;
const extension = (which) =>
	`export default api=>{api.registerTool({name:'duplicate-tool',description:'${which} tool',parameters:{type:'object'},execute:async()=>({content:[]})});api.registerFlag('duplicate-flag',{type:'string',default:'${which}'});api.registerCommand('deploy',{description:'${which} command',handler:async()=>'${which}'});}`;
const cases = [
	{
		name: "no-skills",
		files: { "agent/skills/test-skill.md": skill("test-skill") },
		disabledSkills: true,
	},
	{
		name: "additional-skills",
		files: {
			"agent/skills/test-skill.md": skill("test-skill"),
			"extra/custom.md": skill("custom"),
		},
		disabledSkills: true,
		additionalSkills: ["extra"],
	},
	{
		name: "project-precedence",
		files: {
			"agent/prompts/commit.md": "User prompt",
			"project/.ag1/prompts/commit.md": "Project prompt",
			"agent/skills/collision-skill/SKILL.md": skill("collision-skill"),
			"project/.ag1/skills/collision-skill/SKILL.md": skill("collision-skill"),
			"agent/themes/collision.json": JSON.stringify({
				...theme,
				name: "collision-theme",
			}),
			"project/.ag1/themes/collision.json": JSON.stringify({
				...theme,
				name: "collision-theme",
				vars: { ...theme.vars, accent: "#ff00ff" },
			}),
		},
	},
	{
		name: "auto-overrides",
		files: {
			"agent/settings.json": JSON.stringify({
				extensions: ["-extensions/disabled.ts"],
				skills: ["-skills/skip-skill"],
				prompts: ["-prompts/skip.md"],
				themes: ["-themes/skip.json"],
			}),
			"agent/extensions/disabled.ts": "export default ()=>{}",
			"agent/skills/skip-skill/SKILL.md": skill("skip-skill"),
			"agent/prompts/skip.md": "Skip prompt",
			"agent/themes/skip.json": "{}",
		},
	},
	{
		name: "extension-conflicts",
		files: {
			"agent/extensions/first.ts": extension("first"),
			"agent/extensions/second.ts": extension("second"),
		},
	},
	{
		name: "explicit-extension",
		files: {
			"agent/extensions/global.ts": extension("global"),
			"explicit.ts": extension("explicit"),
		},
		additionalExtensions: ["explicit.ts"],
	},
	{
		name: "user-resources",
		files: {
			"agent/skills/test-skill.md": skill("test-skill"),
			"agent/prompts/test-prompt.md":
				"User prompt without a project collision.",
			"agent/skills/browser-tools/SKILL.md": skill("browser-tools"),
			"agent/skills/browser-tools/EFFICIENCY.md":
				"Extra instructions without frontmatter.",
		},
	},
];
const rows = [];
try {
	for (const input of cases) {
		const base = path.join(owned, input.name),
			cwd = path.join(base, "project"),
			agentDir = path.join(base, "agent");
		fs.mkdirSync(path.join(cwd, ".git"), { recursive: true });
		fs.mkdirSync(agentDir, { recursive: true });
		for (const [p, b] of Object.entries(input.files)) {
			fs.mkdirSync(path.dirname(path.join(base, p)), { recursive: true });
			fs.writeFileSync(path.join(base, p), b);
		}
		const loader = new DefaultResourceLoader({
			cwd,
			agentDir,
			noSkills: input.disabledSkills ?? false,
			additionalSkillPaths: input.additionalSkills?.map((p) =>
				path.join(base, p),
			),
			additionalExtensionPaths: input.additionalExtensions?.map((p) =>
				path.join(base, p),
			),
		});
		await loader.reload();
		const extensions = loader.getExtensions();
		const output = {
			skills: loader.getSkills(),
			prompts: loader.getPrompts(),
			themes: loader
				.getThemes()
				.themes.map((t) => ({ name: t.name, sourcePath: t.sourcePath })),
			extensions: extensions.extensions.map((e) => ({
				path: e.path,
				tools: [...e.tools].map(([name, t]) => ({
					name,
					description: t.definition.description,
				})),
				commands: [...e.commands].map(([name, t]) => ({
					name,
					description: t.description,
				})),
			})),
			errors: extensions.errors,
		};
		rows.push({
			input,
			output: JSON.parse(JSON.stringify(output).replaceAll(base, "/fixture")),
		});
	}
} finally {
	fs.rmSync(owned, { recursive: true, force: true });
}
const out = {
	version: 1,
	source_commit: "e30f0b4b238ee5618928bcd17b4571eb046e7038",
	rows,
};
const target = path.join(root, "fixtures/tools/loader-selection-oracle.json");
if (process.argv.includes("--capture")) {
	if (exists(target)) {
		const prior = JSON.parse(fs.readFileSync(target, "utf8"));
		assert(out.rows.length > prior.rows.length);
		assert.deepEqual(
			{ ...out, rows: out.rows.slice(0, prior.rows.length) },
			prior,
		);
	}
	fs.writeFileSync(target, JSON.stringify(out, null, 2) + "\n");
} else assert.deepEqual(out, JSON.parse(fs.readFileSync(target, "utf8")));
console.log(
	JSON.stringify({
		status: "passed",
		cases: rows.length,
		model_actions: 0,
		network_actions: 0,
	}),
);
