// Deterministic recovery behavior from the immutable SDK; no provider or desktop.
import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { fileURLToPath, pathToFileURL } from "node:url";
await import("../offline_guard.mjs");
const root = path.resolve(
	path.dirname(fileURLToPath(import.meta.url)),
	"../..",
);
const base = path.join(
	root,
	"reference/ag1-executable/packages/coding-agent/dist/core",
);
const { AgentSessionRecoveryArbiter } = await import(
	pathToFileURL(path.join(base, "agent-session-recovery-arbiter.js"))
);
const { AgentSessionRecoveryReplayScheduler } = await import(
	pathToFileURL(path.join(base, "agent-session-recovery-replay-scheduler.js"))
);
const { captureContinueReplaySnapshot, canReplayContinue } = await import(
	pathToFileURL(path.join(base, "agent-session-continue-replay.js"))
);
const originalSet = globalThis.setTimeout,
	originalClear = globalThis.clearTimeout;
let now = 0,
	serial = 0;
const timers = new Map();
globalThis.setTimeout = (callback, delay) => {
	const id = ++serial;
	timers.set(id, { callback, due: now + delay });
	return id;
};
globalThis.clearTimeout = (id) => timers.delete(id);
const rows = [];
const definitions = [
	{
		id: "compaction-blocks-retry",
		steps: [
			{ op: "arm", owner: "compaction" },
			{ op: "schedule", owner: "retry", delay: 0 },
			{ op: "advance", ms: 1 },
		],
	},
	{
		id: "replacement-runs-once",
		steps: [
			{ op: "schedule", owner: "retry", delay: 100 },
			{ op: "advance", ms: 50 },
			{ op: "schedule", owner: "retry", delay: 100 },
			{ op: "advance", ms: 50 },
			{ op: "advance", ms: 50 },
		],
	},
	{
		id: "stale-thinking-skips",
		steps: [
			{ op: "schedule", owner: "compaction", delay: 100 },
			{ op: "change" },
			{ op: "advance", ms: 100 },
		],
	},
	{
		id: "cancel-removes-timer",
		steps: [
			{ op: "schedule", owner: "retry", delay: 100 },
			{ op: "cancel", owner: "retry" },
			{ op: "advance", ms: 100 },
		],
	},
	{
		id: "failed-run-releases-owner",
		failRun: true,
		steps: [
			{ op: "schedule", owner: "retry", delay: 0 },
			{ op: "advance", ms: 1 },
			{ op: "arm", owner: "compaction" },
		],
	},
	{
		id: "same-owner-rearm-and-owner-specific-clear",
		steps: [
			{ op: "arm", owner: "retry" },
			{ op: "arm", owner: "retry" },
			{ op: "arm", owner: "compaction" },
			{ op: "clear", owner: "compaction" },
			{ op: "clear", owner: "retry" },
			{ op: "arm", owner: "compaction" },
		],
	},
];
try {
	for (const definition of definitions) {
		now = 0;
		timers.clear();
		const arbiter = new AgentSessionRecoveryArbiter(),
			events = [],
			errors = [],
			states = [];
		const state = {
			messages: [
				{
					role: "user",
					content: [{ type: "text", text: "hello" }],
					timestamp: 1,
				},
			],
			isStreaming: false,
			model: { provider: "openai", id: "gpt-5.4-mini" },
			thinkingLevel: "medium",
		};
		let runs = 0;
		const schedulers = Object.fromEntries(
			["retry", "compaction"].map((owner) => [
				owner,
				new AgentSessionRecoveryReplayScheduler({
					owner,
					recoveryArbiter: arbiter,
					getState: () => state,
					emit: (event) => events.push(event),
				}),
			]),
		);
		for (const step of definition.steps) {
			let returned = null;
			switch (step.op) {
				case "arm":
					returned = arbiter.tryArm(step.owner);
					break;
				case "clear":
					arbiter.clear(step.owner);
					break;
				case "schedule":
					returned = schedulers[step.owner].schedule({
						delayMs: step.delay,
						alreadyScheduledMessage: "blocked",
						stateChangedMessage: "stale",
						run: async () => {
							runs++;
							if (definition.failRun) throw new Error("fixture run error");
						},
						onRunError: (error) => errors.push(error.message),
					});
					break;
				case "cancel":
					schedulers[step.owner].cancel();
					break;
				case "change":
					state.thinkingLevel = "high";
					break;
				case "advance": {
					now += step.ms;
					for (const [id, timer] of [...timers])
						if (timer.due <= now) {
							timers.delete(id);
							timer.callback();
							await Promise.resolve();
						}
					break;
				}
				default:
					throw new Error("unknown fixture step");
			}
			states.push(
				JSON.parse(
					JSON.stringify({
						returned,
						owner: arbiter.owner ?? null,
						scheduled:
							schedulers.retry.isScheduled || schedulers.compaction.isScheduled,
						runs,
						events,
						errors,
					}),
				),
			);
		}
		rows.push({ ...definition, states });
	}
} finally {
	globalThis.setTimeout = originalSet;
	globalThis.clearTimeout = originalClear;
}
const replay = [];
for (const change of [
	"none",
	"model",
	"provider",
	"thinking",
	"append",
	"replace-tail",
	"streaming",
	"empty",
]) {
	const state = {
		messages:
			change === "empty"
				? []
				: [{ role: "user", content: "hello", timestamp: 1 }],
		isStreaming: false,
		model: { provider: "openai", id: "gpt-5.4-mini" },
		thinkingLevel: "medium",
	};
	const snapshot = captureContinueReplaySnapshot(state);
	if (change === "model") state.model.id = "gpt-5.4";
	if (change === "provider") state.model.provider = "fixture-provider";
	if (change === "thinking") state.thinkingLevel = "high";
	if (change === "append")
		state.messages.push({ role: "user", content: "next", timestamp: 2 });
	if (change === "replace-tail") state.messages[0] = { ...state.messages[0] };
	if (change === "streaming") state.isStreaming = true;
	replay.push({ change, accepted: canReplayContinue(state, snapshot) });
}
const source_files = [
	"agent-session-recovery-arbiter.ts",
	"agent-session-recovery-replay-scheduler.ts",
	"agent-session-continue-replay.ts",
].map((name) => {
	const relative = "packages/coding-agent/src/core/" + name;
	return {
		path: relative,
		sha256: createHash("sha256")
			.update(
				fs.readFileSync(path.join(root, "reference/ag1-executable", relative)),
			)
			.digest("hex"),
	};
});
const result = {
	version: 1,
	source_commit: "e30f0b4b238ee5618928bcd17b4571eb046e7038",
	source_files,
	rows,
	replay,
	model_actions: 0,
	network_actions: 0,
};
const target = path.join(root, "fixtures/runtime/recovery-oracle.json");
if (process.argv.includes("--capture")) {
	assert(!fs.existsSync(target));
	fs.writeFileSync(target, JSON.stringify(result, null, 2) + "\n");
} else assert.deepEqual(result, JSON.parse(fs.readFileSync(target, "utf8")));
console.log(
	JSON.stringify({
		status: "passed",
		recovery_scenarios: rows.length,
		replay_cases: replay.length,
		model_actions: 0,
		network_actions: 0,
	}),
);
