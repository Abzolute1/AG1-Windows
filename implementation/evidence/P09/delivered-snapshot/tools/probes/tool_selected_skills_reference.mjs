import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { syncBuiltinESMExports } from "node:module";
import { fileURLToPath, pathToFileURL } from "node:url";
await import("../offline_guard.mjs");
const root = path.resolve(
	path.dirname(fileURLToPath(import.meta.url)),
	"../..",
);
const reference = path.join(root, "reference/ag1-executable");
const owned = fs.mkdtempSync(path.join(root, ".local/p09-selected-skills-"));
// Isolate the SDK's home lookup without changing HOME or reading user skills.
os.homedir = () => path.join(owned, "home");
syncBuiltinESMExports();
const sdk = await import(
	pathToFileURL(
		path.join(reference, "packages/coding-agent/dist/core/skills.js"),
	)
);
const files = {},
	hashes = {},
	rows = [];
const normalize = (v) =>
	JSON.parse(JSON.stringify(v).replaceAll(owned, "/fixture"));
function copyTree(relative) {
	for (const entry of fs.readdirSync(path.join(reference, relative), {
		withFileTypes: true,
	})) {
		const source = relative + "/" + entry.name;
		if (entry.isDirectory()) copyTree(source);
		else if (entry.isFile()) {
			const content = fs.readFileSync(path.join(reference, source), "utf8");
			const target = source.replace("packages/coding-agent/test/fixtures/", "");
			files[target] = content;
			hashes[source] = createHash("sha256").update(content).digest("hex");
			fs.mkdirSync(path.dirname(path.join(owned, target)), { recursive: true });
			fs.writeFileSync(path.join(owned, target), content);
		}
	}
}
try {
	copyTree("packages/coding-agent/test/fixtures/skills");
	copyTree("packages/coding-agent/test/fixtures/skills-collision");
	const dirs = fs
		.readdirSync(path.join(owned, "skills"))
		.sort()
		.map((name) => "skills/" + name);
	for (const dir of [...dirs, "skills", "missing"]) {
		const output = sdk.loadSkillsFromDir({
			dir: path.join(owned, dir),
			source: "test",
		});
		for (const diagnostic of output.diagnostics) {
			if (diagnostic.path.endsWith("/invalid-yaml/SKILL.md")) {
				assert(diagnostic.message.includes("at line"));
				diagnostic.message = "<invalid YAML with location>";
			}
		}
		rows.push({ kind: "directory", path: dir, output });
	}
	const cwd = path.join(owned, "cwd"),
		agentDir = path.join(owned, "agent");
	fs.mkdirSync(cwd);
	fs.mkdirSync(agentDir);
	const homeSkill = "home/.ag1/agent/skills/home-skill/SKILL.md";
	files[homeSkill] =
		"---\nname: home-skill\ndescription: Fixture home skill.\n---\nHome body.";
	fs.mkdirSync(path.dirname(path.join(owned, homeSkill)), { recursive: true });
	fs.writeFileSync(path.join(owned, homeSkill), files[homeSkill]);
	for (const paths of [
		["skills/valid-skill"],
		["missing"],
		["~/.ag1/agent/skills"],
		["home/.ag1/agent/skills"],
		["skills-collision/first", "skills-collision/second"],
	]) {
		const skillPaths = paths.map((p) =>
			p.startsWith("~") ? p : path.join(owned, p),
		);
		rows.push({
			kind: "explicit",
			paths,
			output: sdk.loadSkills({
				cwd,
				agentDir,
				includeDefaults: false,
				skillPaths,
			}),
		});
	}
	const skill = (name, description, options = {}) => ({
		name,
		description,
		filePath: `/skills/${name}/SKILL.md`,
		baseDir: `/skills/${name}`,
		disableModelInvocation: false,
		triggers: [],
		...options,
	});
	const skills = [
		skill("review-diff", "Review code changes", {
			triggers: ["review changes", "code review"],
		}),
		skill("tdd", "Test-driven development", {
			triggers: ["test driven development", "write test first"],
		}),
	];
	for (const prompt of [
		"Please review this diff",
		"Can you perform a code review?",
		"Use test driven development for this",
		"Run the tests and review the output",
	])
		rows.push({
			kind: "match",
			prompt,
			skills,
			output: sdk.matchSkillForPrompt(prompt, skills),
		});
	for (const skills of [
		[],
		[skill("test-skill", "A test skill.")],
		[skill("test-skill", 'A skill with <special> & "characters".')],
		[skill("skill-one", "First skill."), skill("skill-two", "Second skill.")],
		[
			skill("visible-skill", "A visible skill."),
			skill("hidden-skill", "A hidden skill.", {
				disableModelInvocation: true,
			}),
		],
		[
			skill("hidden-skill", "A hidden skill.", {
				disableModelInvocation: true,
			}),
		],
	])
		rows.push({
			kind: "format",
			skills,
			output: sdk.formatSkillsForPrompt(skills),
		});
} finally {
	fs.rmSync(owned, { recursive: true, force: true });
}
const out = normalize({
	version: 1,
	source_commit: "e30f0b4b238ee5618928bcd17b4571eb046e7038",
	source_hashes: hashes,
	files,
	rows,
});
const target = path.join(root, "fixtures/tools/selected-skills-oracle.json");
if (process.argv.includes("--capture")) {
	assert(!fs.existsSync(target));
	fs.writeFileSync(target, JSON.stringify(out, null, 2) + "\n");
} else assert.deepEqual(out, JSON.parse(fs.readFileSync(target, "utf8")));
console.log(
	JSON.stringify({
		status: "passed",
		cases: rows.length,
		reference_files: Object.keys(hashes).length,
		model_actions: 0,
		network_actions: 0,
	}),
);
