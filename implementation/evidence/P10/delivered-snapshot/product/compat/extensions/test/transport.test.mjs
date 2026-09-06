import test from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { createHash } from "node:crypto";
import { fileURLToPath } from "node:url";
import { SnapshotTransfer } from "../snapshot-transfer.mjs";

test("snapshot transfer rejects missing, corrupt, reordered and reused data", () => {
	const bytes = Buffer.from(JSON.stringify({ session: { id: "native" } }));
	const metadata = { id: "snapshot-1", bytes: bytes.length, sha256: createHash("sha256").update(bytes).digest("hex") };
	const transfer = new SnapshotTransfer();
	transfer.begin(metadata);
	assert.throws(() => transfer.take(metadata.id), /Incomplete/);
	transfer.begin(metadata);
	assert.throws(() => transfer.append({ id: metadata.id, offset: 1, data: bytes.toString("base64") }), /Invalid/);
	assert.throws(() => transfer.take(metadata.id), /Incomplete/);
	transfer.begin({ ...metadata, sha256: "0".repeat(64) });
	transfer.append({ id: metadata.id, offset: 0, data: bytes.toString("base64") });
	assert.throws(() => transfer.take(metadata.id), /Incomplete/);
	transfer.begin(metadata);
	transfer.append({ id: metadata.id, offset: 0, data: bytes.toString("base64") });
	assert.deepEqual(transfer.take(metadata.id), { session: { id: "native" } });
	assert.throws(() => transfer.take(metadata.id), /Incomplete/);
	assert.throws(() => transfer.begin({ ...metadata, bytes: 8 * 1024 * 1024 + 1 }), /Invalid/);
});
function child(t, code, bridge = async () => undefined) {
	const root = fs.mkdtempSync(
		path.join(os.tmpdir(), "ag1-extension-transport-"),
	);
	const entry = path.join(root, "extension.ts");
	fs.writeFileSync(entry, code);
	const host = fileURLToPath(new URL("../host.mjs", import.meta.url));
	const process = spawn(
		globalThis.process.execPath,
		["--max-old-space-size=128", host],
		{ cwd: root, env: {}, stdio: ["pipe", "pipe", "pipe"] },
	);
	let stderr = "",
		buffer = "",
		id = 0;
	const calls = [],
		events = [],
		pending = new Map();
	process.stderr.on("data", (chunk) => {
		stderr = (stderr + chunk).slice(-65536);
	});
	const send = (value) => process.stdin.write(JSON.stringify(value) + "\n");
	process.stdout.on("data", (chunk) => {
		buffer += chunk;
		while (buffer.includes("\n")) {
			const end = buffer.indexOf("\n"),
				frame = JSON.parse(buffer.slice(0, end));
			buffer = buffer.slice(end + 1);
			if (frame.kind === "response" || frame.kind === "invoke_response") {
				const p = pending.get(frame.id);
				assert(p, `unexpected response ${JSON.stringify(frame)}`);
				pending.delete(frame.id);
				if (frame.error)
					p.reject(
						Object.assign(new Error(frame.error.message), {
							code: frame.error.code,
						}),
					);
				else p.resolve(frame.result.defined ? frame.result.value : undefined);
			} else if (frame.kind === "event") events.push(frame.event);
			else if (frame.kind === "call") {
				calls.push(frame);
				Promise.resolve(bridge(frame)).then(
					(value) =>
						send({
							v: 1,
							kind: "response",
							id: frame.id,
							result:
								value === undefined
									? { defined: false }
									: { defined: true, value },
						}),
					(error) =>
						send({
							v: 1,
							kind: "response",
							id: frame.id,
							error: { message: error.message },
						}),
				);
			} else if (frame.kind === "call_cancel") {
				assert(
					calls.some((call) => call.id === frame.id),
					"cancel belongs to an observed native callback",
				);
			} else assert.fail(`unexpected packet ${JSON.stringify(frame)}`);
		}
	});
	process.on("exit", () => {
		for (const p of pending.values())
			p.reject(new Error(`owned host exited: ${stderr}`));
		pending.clear();
	});
	t.after(async () => {
		if (process.exitCode === null && process.signalCode === null) {
			process.kill("SIGKILL");
			await once(process, "exit");
		}
		fs.rmSync(root, { recursive: true, force: true });
	});
	const request = (method, args) => {
		const requestId = `request-${++id}`;
		return {
			id: requestId,
			promise: new Promise((resolve, reject) => {
				pending.set(requestId, { resolve, reject });
				send({ v: 1, kind: "request", id: requestId, method, args });
			}),
		};
	};
	return {
		root,
		entry,
		process,
		send,
		calls,
		events,
		request,
		invoke(parent, args) {
			const invocationId = `invoke-${++id}`;
			return {
				id: invocationId,
				promise: new Promise((resolve, reject) => {
					pending.set(invocationId, { resolve, reject });
					send({
						v: 1,
						kind: "invoke",
						id: invocationId,
						parent,
						method: "provider",
						args,
					});
				}),
			};
		},
		stderr: () => stderr,
		async start() {
			await request("initialize", { cwd: root }).promise;
			await request("load", {
				entries: [
					{
						path: entry,
						sha256: createHash("sha256").update(code).digest("hex"),
					},
				],
			}).promise;
			await request("bind", { snapshot: { hasUI: true } }).promise;
		},
	};
}
test("private transport separates logs and multiplexes acknowledged native callbacks", async (t) => {
	const f = child(
		t,
		`export default api=>{console.log('fixture log');api.registerCommand('choose',{handler:async(args,ctx)=>{api.appendEntry('custom',{unknown:['kept']});return await ctx.ui.select('Title',['a','b']);}});}`,
		async (frame) => (frame.method === "ui.select" ? "b" : undefined),
	);
	await f.start();
	assert.equal(
		await f.request("command", { name: "choose", arguments: "" }).promise,
		"b",
	);
	assert.deepEqual(
		f.calls.map((c) => c.method),
		["session.appendEntry", "ui.select"],
	);
	assert(f.stderr().includes("fixture log"));
	assert(f.calls.every((c) => c.operation === "request-4"));
	await assert.rejects(
		f.request("not-supported", {}).promise,
		(error) => error.code === "unsupported_api",
	);
	f.process.stdin.end();
	const [code] = await once(f.process, "exit");
	assert.equal(code, 0);
});
test("cooperative cancel settles an operation and an uncooperative host remains killable by its owner", async (t) => {
	const f = child(
		t,
		`export default api=>{api.registerCommand('wait',{handler:async(args,ctx)=>new Promise(resolve=>{ctx.ui.notify('ready');ctx.signal.addEventListener('abort',()=>resolve('cancelled'));})});api.registerCommand('hang',{handler:async()=>{while(true){}}});}`,
	);
	await f.start();
	const waiting = f.request("command", { name: "wait", arguments: "" });
	while (!f.calls.length)
		await new Promise((resolve) => setTimeout(resolve, 5));
	f.send({ v: 1, kind: "cancel", id: waiting.id });
	assert.equal(await waiting.promise, "cancelled");
	const hanging = f.request("command", { name: "hang", arguments: "" });
	const rejected = assert.rejects(hanging.promise, /owned host exited/);
	await new Promise((resolve) => setTimeout(resolve, 30));
	f.process.kill("SIGKILL");
	await rejected;
	assert.equal(f.process.signalCode, "SIGKILL");
});
test("oversized and truncated protocol frames fail closed", async (t) => {
	const f = child(t, "export default ()=>{}");
	const exited = once(f.process, "exit");
	f.process.stdin.on("error", () => {});
	f.process.stdin.write("x".repeat(1024 * 1024 + 1));
	const [code] = await exited;
	assert.equal(code, 70);
	assert(f.stderr().includes("frame quota"));
	const g = child(t, "export default ()=>{}");
	const ended = once(g.process, "exit");
	g.process.stdin.end('{"v":1');
	const [second] = await ended;
	assert.equal(second, 70);
	assert(g.stderr().includes("Truncated"));
});

test("native callbacks invoke staged provider hooks in the same host with tagged nested callbacks", async (t) => {
	let f, registration, invocation;
	f = child(
		t,
		`export default api=>{
		let calls=0;
		api.registerProvider('fixture',{api:'custom',oauth:{name:'Fixture',modifyModels:async models=>{
			calls++;await api.setModel({provider:'fixture',id:'model'});return models.map(m=>({...m,calls}));
		}}});
		api.registerCommand('again',{handler:()=>api.setThinkingLevel('high')});
	}`,
		async (frame) => {
			if (frame.method === "provider.register") {
				registration = frame.args.registration;
				const request = f.invoke(frame.id, {
					name: "fixture",
					registration,
					hook: "oauth.modifyModels",
					arguments: { models: [{ id: "one" }], credentials: {} },
				});
				invocation = request.id;
				assert.deepEqual(await request.promise, [{ id: "one", calls: 1 }]);
				return { retainRegistrations: [registration] };
			}
			if (frame.method === "model.set") {
				assert.equal(frame.invocation, invocation);
				return true;
			}
			if (frame.method === "session.setThinkingLevel") {
				assert.equal(frame.invocation, undefined);
				const request = f.invoke(frame.id, {
					name: "fixture",
					registration,
					hook: "oauth.modifyModels",
					arguments: { models: [{ id: "two" }], credentials: {} },
				});
				invocation = request.id;
				assert.deepEqual(await request.promise, [{ id: "two", calls: 2 }]);
				return;
			}
			assert.fail(`unexpected method ${frame.method}`);
		},
	);
	await f.start();
	await f.request("command", { name: "again", arguments: "" }).promise;
	assert.equal(f.calls.filter((c) => c.method === "model.set").length, 2);
});

test("synchronous provider callbacks consume large native acknowledgements and keep the extension stack current", async (t) => {
	let accepted;
	const f = child(
		t,
		`export default api=>{api.registerCommand('change',{handler:(_,ctx)=>{
		api.registerProvider('fixture',{api:'custom'});
		const model=ctx.modelRegistry.find('fixture','large');
		api.unregisterProvider('fixture');
		return {length:model.name.length,removed:ctx.modelRegistry.find('fixture','large')===undefined};
	}});}`,
		async (frame) => {
			if (frame.method === "provider.register") {
				await new Promise((resolve) => setTimeout(resolve, 20));
				accepted = frame.args.registration;
				return {
					models: [
						{ provider: "fixture", id: "large", name: "x".repeat(240000) },
					],
					retainRegistrations: [accepted],
				};
			}
			assert.equal(frame.method, "provider.unregister");
			return { models: [], retainRegistrations: [] };
		},
	);
	await f.start();
	assert.deepEqual(
		await f.request("command", { name: "change", arguments: "" }).promise,
		{ length: 240000, removed: true },
	);
	assert(accepted);
});

test("cancellation crosses a synchronous native callback and a rejected acknowledgement remains catchable", async (t) => {
	let f;
	f = child(
		t,
		`export default api=>{api.registerCommand('change',{handler:(_,ctx)=>{
		let rejected=false;try {api.registerProvider('fixture',{api:'custom'})}catch{rejected=true}
		return {rejected,cancelled:ctx.signal.aborted,absent:ctx.modelRegistry.find('fixture','model')===undefined};
	}});api.registerCommand('alive',{handler:()=>true})}`,
		async (frame) => {
			assert.equal(frame.method, "provider.register");
			f.send({ v: 1, kind: "cancel", id: frame.operation });
			await new Promise((resolve) => setTimeout(resolve, 10));
			throw new Error("native operation cancelled");
		},
	);
	await f.start();
	assert.deepEqual(
		await f.request("command", { name: "change", arguments: "" }).promise,
		{ rejected: true, cancelled: true, absent: true },
	);
	assert.equal(
		await f.request("command", { name: "alive", arguments: "" }).promise,
		true,
	);
});

test(
	"invalid native responses fail closed while synchronous registration is waiting",
	{ timeout: 5000 },
	async (t) => {
		let f;
		f = child(
			t,
			`export default api=>{api.registerCommand('change',{handler:()=>api.registerProvider('fixture',{api:'custom'})})}`,
			(frame) => {
				f.send({
					v: 1,
					kind: "response",
					id: "unmatched-response",
					result: { defined: false },
				});
				return new Promise(() => {});
			},
		);
		await f.start();
		const exited = once(f.process, "exit");
		await assert.rejects(
			f.request("command", { name: "change", arguments: "" }).promise,
			/owned host exited/,
		);
		const [code] = await exited;
		assert.equal(code, 70);
		assert(f.stderr().includes("Unmatched or duplicate native response"));
	},
);
