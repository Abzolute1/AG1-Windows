import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { createHash } from "node:crypto";
import { ExtensionRuntime, UnsupportedAPI, bounded } from "../runtime.mjs";
function fixture(t, sources, bridge = async () => ({})) {
	const root = fs.mkdtempSync(path.join(os.tmpdir(), "ag1-extension-fixture-"));
	t.after(() => fs.rmSync(root, { recursive: true, force: true }));
	const entries = sources.map((code, i) => {
		const target = path.join(root, `extension-${i}.ts`);
		fs.writeFileSync(target, code);
		return {
			path: target,
			sha256: createHash("sha256").update(code).digest("hex"),
			sourceInfo: {
				path: target,
				source: "fixture",
				scope: "temporary",
				origin: "top-level",
				baseDir: root,
			},
		};
	});
	const events = [];
	const runtime = new ExtensionRuntime(bridge, {
		cwd: root,
		emit: (event) => events.push(event),
	});
	return { root, entries, runtime, events };
}
test("owned TypeScript extensions load TypeBox tools, local dependencies and collision metadata", async (t) => {
	const f = fixture(t, [
		`import {Type} from '@sinclair/typebox';import {value} from './value.js';export default function(api){api.registerTool({name:'sample',label:'Sample',description:'first',parameters:Type.Object({n:Type.Number()}),prepareArguments:a=>({n:Number(a.n)}),execute:async(id,args,signal,update,ctx)=>{update({content:[{type:'text',text:'progress'}]});return {content:[{type:'text',text:String(args.n+value)}],details:{cwd:ctx.cwd,signal:!!signal,custom:{unknown:true}}};}});api.registerCommand('shared',{description:'first',handler:async()=> 'first'});api.registerFlag('flag',{type:'string',default:'first'});}`,
		`export default function(api){api.registerTool({name:'sample',description:'second',parameters:{type:'object'},execute:async()=>({content:[]})});api.registerCommand('shared',{description:'second',handler:async()=> 'second'});api.registerFlag('flag',{type:'string',default:'second'});}`,
	]);
	fs.writeFileSync(path.join(f.root, "value.js"), "export const value=7;");
	const catalog = await f.runtime.load(f.entries);
	assert.deepEqual(catalog.errors, []);
	assert.equal(catalog.tools.length, 1);
	assert.equal(catalog.tools[0].description, "first");
	assert.deepEqual(catalog.tools[0].parameters.required, ["n"]);
	assert.deepEqual(
		catalog.commands.map((c) => c.invocationName),
		["shared:1", "shared:2"],
	);
	assert.equal(catalog.flags.length, 1);
	await f.runtime.bind({ isIdle: true });
	const args = f.runtime.prepareTool("sample", { n: "4" });
	assert.deepEqual(args, { n: 4 });
	const result = await f.runtime.invoke("tool", () =>
		f.runtime.executeTool("sample", "call", args),
	);
	assert.equal(result.content[0].text, "11");
	assert.equal(result.details.signal, true);
	assert.deepEqual(result.details.custom, { unknown: true });
	assert.equal(f.events[0].type, "tool_update");
	assert.equal(
		await f.runtime.invoke("command", () => f.runtime.command("shared:2", "")),
		"second",
	);
});
test("input transforms and tool result patches chain while throwing handlers are isolated", async (t) => {
	const f = fixture(t, [
		`export default api=>{api.on('input',event=>({action:'transform',text:event.text+' one'}));api.on('tool_result',()=>({content:[{type:'text',text:'first'}]}));}`,
		`export default api=>{api.on('input',()=>{throw new Error('isolated')});api.on('input',event=>({action:'transform',text:event.text+' two'}));api.on('tool_result',event=>({details:{content:event.content},isError:false}));}`,
	]);
	await f.runtime.load(f.entries);
	await f.runtime.bind({});
	const images = [{ type: "image", data: "AA==", mimeType: "image/png" }];
	const input = await f.runtime.invoke("input", () =>
		f.runtime.event({ type: "input", text: "base", images, source: "rpc" }),
	);
	assert.deepEqual(input, {
		action: "transform",
		text: "base one two",
		images,
	});
	assert.equal(f.runtime.errors.length, 1);
	const result = await f.runtime.invoke("result", () =>
		f.runtime.event({
			type: "tool_result",
			toolName: "sample",
			content: [{ type: "text", text: "original" }],
			isError: true,
		}),
	);
	assert.deepEqual(result, {
		content: [{ type: "text", text: "first" }],
		details: { content: [{ type: "text", text: "first" }] },
		isError: false,
	});
});
test("native requests are acknowledged, carry operation identity, and retain unknown custom records", async (t) => {
	const calls = [];
	const f = fixture(
		t,
		[
			`import {complete} from '@ag1/ai';export default api=>{api.registerCommand('request',{handler:async(args,ctx)=>{api.appendEntry('unknown.record',{arbitrary:[1,2]});const selected=await ctx.ui.select('Choose',['one','two']);const response=await complete({provider:'fixture',id:'model'},{messages:[]});return {selected,response,hasHost:!!ctx.host};}});}`,
		],
		async (request) => {
			calls.push(request);
			return request.method === "ui.select"
				? "two"
				: request.method === "inference.complete"
					? { role: "assistant", content: [{ type: "text", text: "brokered" }] }
					: undefined;
		},
	);
	await f.runtime.load(f.entries);
	await f.runtime.bind({ hasUI: true, hasHost: true });
	const result = await f.runtime.invoke("request-op", () =>
		f.runtime.command("request", ""),
	);
	assert.equal(result.selected, "two");
	assert.equal(result.response.content[0].text, "brokered");
	assert.deepEqual(
		calls.map((c) => c.method),
		["session.appendEntry", "ui.select", "inference.complete"],
	);
	assert(calls.every((c) => c.operation === "request-op"));
	assert.deepEqual(calls[0].args.args, [
		"unknown.record",
		{ arbitrary: [1, 2] },
	]);
});
test("load errors, stale grants, unsupported APIs and structural limits are explicit", async (t) => {
	const f = fixture(t, [
		`export default api=>api.appendEntry('cannot-during-load',{});`,
		`export const missing=1;`,
		`export default api=>api.noSuchAPI();`,
	]);
	const result = await f.runtime.load(f.entries);
	assert.equal(result.extensions.length, 0);
	assert.equal(result.errors.length, 3);
	assert(result.errors[2].error.includes("Unsupported extension API"));
	const stale = fixture(t, ["export default ()=>{}"]);
	fs.writeFileSync(
		stale.entries[0].path,
		'export default ()=>{throw new Error("changed")};',
	);
	await assert.rejects(stale.runtime.load(stale.entries), /Stale/);
	const cycle = {};
	cycle.self = cycle;
	assert.throws(() => bounded(cycle), /structural/);
	assert.throws(() => bounded(() => {}), /data only/);
	for (const value of [NaN, Infinity, -Infinity])
		assert.throws(() => bounded({ value }), /non-finite/);
	assert.equal(new UnsupportedAPI("test").code, "unsupported_api");
});
test("queued provider hooks register through native authority and support stream and OAuth invocation", async (t) => {
	const calls = [];
	const f = fixture(
		t,
		[
			`export default api=>{api.registerProvider('removed',{api:'old'});api.unregisterProvider('removed');api.registerProvider('bad',undefined);api.registerProvider('custom',{api:'fixture',streamSimple:()=>({async *[Symbol.asyncIterator](){yield {type:'text_delta',delta:'custom'};yield {type:'done'};},async result(){return {role:'assistant',content:[{type:'text',text:'custom'}]};}}),oauth:{name:'Fixture',login:async cb=>{cb.onAuth({url:'https://fixture.invalid/auth'});return {access:await cb.onPrompt({message:'code'})};},refreshToken:async c=>({...c,refreshed:true}),getApiKey:c=>c.access,modifyModels:models=>models.map(m=>({...m,name:'modified'}))}});}`,
		],
		async (request) => {
			calls.push(request);
			return request.method === "oauth.onPrompt" ? "fixture-code" : undefined;
		},
	);
	await f.runtime.load(f.entries);
	assert.equal(calls.length, 0);
	await f.runtime.bind({});
	assert.equal(f.runtime.errors.length, 1);
	assert.equal(calls[0].args.name, "custom");
	assert.equal(calls[0].args.config.hooks.streamSimple, true);
	assert(!calls.some((c) => c.args.name === "removed"));
	const result = await f.runtime.invoke("stream", () =>
		f.runtime.provider("custom", "streamSimple", {
			model: { id: "fixture" },
			context: { messages: [] },
			options: {},
		}),
	);
	assert.equal(result.content[0].text, "custom");
	assert.equal(f.events.filter((e) => e.type === "provider_event").length, 2);
	const credentials = await f.runtime.invoke("login", () =>
		f.runtime.provider("custom", "oauth.login", {}),
	);
	assert.deepEqual(credentials, { access: "fixture-code" });
	assert.equal(
		await f.runtime.invoke("key", () =>
			f.runtime.provider("custom", "oauth.getApiKey", { credentials }),
		),
		"fixture-code",
	);
});
test("void native actions settle before a failing command returns and cancellation reaches its signal", async (t) => {
	let ack = false;
	const f = fixture(
		t,
		[
			`export default api=>{api.registerCommand('fail',{handler:async()=>{api.appendEntry('record',{value:1});throw new Error('command failed');}});api.registerCommand('wait',{handler:async(args,ctx)=>new Promise(resolve=>ctx.signal.addEventListener('abort',()=>resolve('cancelled'),{once:true}))});}`,
		],
		async () => {
			await new Promise((r) => setTimeout(r, 10));
			ack = true;
		},
	);
	await f.runtime.load(f.entries);
	await f.runtime.bind({});
	await assert.rejects(
		f.runtime.invoke("failed-command", () => f.runtime.command("fail", "")),
		/command failed/,
	);
	assert(ack);
	assert.equal(f.runtime.pending.length, 0);
	const waiting = f.runtime.invoke("waiting", () =>
		f.runtime.command("wait", ""),
	);
	f.runtime.cancel("wrong-operation");
	assert.equal(f.runtime.current.signal.aborted, false);
	f.runtime.cancel("waiting");
	assert.equal(await waiting, "cancelled");
});

test("pure hooks match the pinned SDK runner oracle including short circuit and undefined semantics", async (t) => {
	const oracle = JSON.parse(
		fs.readFileSync(
			new URL(
				"../../../../fixtures/tools/extension-runner-oracle.json",
				import.meta.url,
			),
			"utf8",
		),
	);
	for (const row of oracle.rows) {
		const f = fixture(t, row.sources);
		await f.runtime.load(f.entries);
		assert.deepEqual(f.runtime.errors, [], row.id);
		await f.runtime.bind({});
		const value = await f.runtime.invoke(row.id, () =>
			f.runtime.event(structuredClone(row.event)),
		);
		const packet =
			value === undefined ? { defined: false } : { defined: true, value };
		const normalize = (v) =>
			JSON.parse(JSON.stringify(v).replaceAll(f.root, "/fixture"));
		assert.deepEqual(normalize(packet), row.output, row.id);
		assert.deepEqual(
			normalize(f.runtime.errors),
			row.errors,
			`${row.id}: errors`,
		);
		assert.equal(
			f.runtime.catalog().handlers.includes(row.event.type),
			row.hasHandlers,
			row.id,
		);
	}
});

test("tool, command, flag and shortcut collections match the SDK precedence oracle", async (t) => {
	const oracle = JSON.parse(
		fs.readFileSync(
			new URL(
				"../../../../fixtures/tools/extension-collections-oracle.json",
				import.meta.url,
			),
			"utf8",
		),
	);
	for (const row of oracle.rows) {
		const f = fixture(t, row.sources);
		await f.runtime.load(f.entries);
		await f.runtime.bind({});
		assert.deepEqual(f.runtime.errors, [], row.id);
		const catalog = f.runtime.catalog();
		const tools = catalog.tools.map(({ name, description }) => ({
			name,
			description,
		}));
		const commands = catalog.commands.map(
			({ name, invocationName, description }) => ({
				name,
				invocationName,
				description,
			}),
		);
		const commandResults = [];
		for (const c of catalog.commands)
			commandResults.push({
				name: c.invocationName,
				result: await f.runtime.invoke("command", () =>
					f.runtime.command(c.invocationName, ""),
				),
			});
		const flags = catalog.flags,
			defaults = f.runtime.getFlagValues();
		f.runtime.setFlagValue("shared", "overridden");
		f.runtime.setFlagValue("unknown", true);
		const values = f.runtime.getFlagValues();
		f.runtime.getFlagValues().shared = "mutated-copy";
		assert.equal(f.runtime.getFlagValues().shared, "overridden");
		const updatedFlags = catalog.commands.some((c) => c.name === "flags")
			? await f.runtime.invoke("flags", () => f.runtime.command("flags", ""))
			: null;
		const { shortcuts, diagnostics } = f.runtime.shortcuts(row.keybindings);
		const renderer = f.runtime.extensions
			.map((e) => e.renderers.get("record"))
			.find(Boolean);
		const output = {
			tools,
			commands,
			commandResults,
			flags,
			defaults,
			values,
			updatedFlags,
			shortcuts,
			diagnostics,
			renderer: renderer ? renderer({}, {}, {}) : null,
		};
		assert.deepEqual(
			JSON.parse(JSON.stringify(output).replaceAll(f.root, "/fixture")),
			row.output,
			row.id,
		);
		for (const s of shortcuts) {
			assert.equal(
				await f.runtime.invoke("shortcut", () =>
					f.runtime.shortcut(s.key, row.keybindings),
				),
				s.description === "last"
					? "last"
					: s.description === "new"
						? "new"
						: s.shortcut,
			);
		}
		await assert.rejects(
			f.runtime.invoke("reserved", () =>
				f.runtime.shortcut("ctrl+c", row.keybindings),
			),
			/Unknown or reserved/,
		);
	}
});

test("provider hook versions survive settings replacement and rejected native changes", async (t) => {
	const registrations = [];
	let rejectRemove = true;
	const f = fixture(
		t,
		[
			`export default api=>{
		const streamSimple=()=>({async *[Symbol.asyncIterator](){yield {type:'done'}},async result(){return {version:1}}});
		api.registerProvider('custom',{api:'fixture',streamSimple});
		api.registerCommand('replace',{handler:()=>api.registerProvider('custom',{api:'fixture'})});
		api.registerCommand('reject',{handler:()=>api.registerProvider('custom',{api:'fixture',reject:true,streamSimple})});
		api.registerCommand('remove',{handler:()=>api.unregisterProvider('custom')});
	}`,
		],
		async (request) => {
			if (request.method === "provider.register") {
				if (request.args.config.reject)
					throw new Error("native registration rejected");
				registrations.push(request.args.registration);
				return { retainRegistrations: [...registrations] };
			}
			if (request.method === "provider.unregister") {
				if (rejectRemove) throw new Error("native removal rejected");
				return { retainRegistrations: [] };
			}
		},
	);
	await f.runtime.load(f.entries);
	await f.runtime.bind({});
	await f.runtime.invoke("replace", () => f.runtime.command("replace", ""));
	assert.equal(registrations.length, 2);
	await assert.rejects(
		f.runtime.invoke("current", () =>
			f.runtime.provider("custom", "streamSimple", {}, registrations[1]),
		),
		/streamSimple/,
	);
	assert.deepEqual(
		await f.runtime.invoke("retained", () =>
			f.runtime.provider("custom", "streamSimple", {}, registrations[0]),
		),
		{ version: 1 },
	);
	await assert.rejects(
		f.runtime.invoke("reject", () => f.runtime.command("reject", "")),
		/native registration rejected/,
	);
	assert.equal(f.runtime.providerVersions.size, 2);
	await assert.rejects(
		f.runtime.invoke("remove-reject", () => f.runtime.command("remove", "")),
		/native removal rejected/,
	);
	assert(f.runtime.providers.has("custom"));
	rejectRemove = false;
	await f.runtime.invoke("remove", () => f.runtime.command("remove", ""));
	assert.equal(f.runtime.providers.size, 0);
	assert.equal(f.runtime.providerVersions.size, 0);
	await assert.rejects(
		f.runtime.invoke("retired", () =>
			f.runtime.provider("custom", "streamSimple", {}, registrations[0]),
		),
		/Unknown extension provider/,
	);
});

test("native registration can validate a staged OAuth hook before acknowledging it", async (t) => {
	let f;
	f = fixture(
		t,
		[
			`export default api=>api.registerProvider('custom',{api:'fixture',oauth:{name:'Fixture',modifyModels:models=>models.map(m=>({...m,name:'staged'}))}});`,
		],
		async (request) => {
			assert.equal(request.method, "provider.register");
			assert(!f.runtime.providers.has("custom"));
			assert.deepEqual(
				await f.runtime.provider(
					"custom",
					"oauth.modifyModels",
					{ models: [{ id: "one" }], credentials: {} },
					request.args.registration,
				),
				[{ id: "one", name: "staged" }],
			);
			return { retainRegistrations: [request.args.registration] };
		},
	);
	await f.runtime.load(f.entries);
	await f.runtime.bind({});
	assert.equal(f.runtime.providerVersions.size, 1);
});

test("a rejected resource handler contributes no partial paths and later handlers still run", async (t) => {
	const f = fixture(t, [
		`export default api=>{
    api.on('resources_discover',()=>({skillPaths:['prior']}));
    api.on('resources_discover',()=>({skillPaths:['partial'],promptPaths:Array(257).fill('overflow')}));
    api.on('resources_discover',()=>({promptPaths:['later']}));
  }`,
	]);
	await f.runtime.load(f.entries);
	await f.runtime.bind({});
	const result = await f.runtime.invoke("resources", () =>
		f.runtime.event({
			type: "resources_discover",
			cwd: f.root,
			reason: "reload",
		}),
	);
	assert.deepEqual(result, {
		skillPaths: [{ path: "prior", extensionPath: f.entries[0].path }],
		promptPaths: [{ path: "later", extensionPath: f.entries[0].path }],
		themePaths: [],
	});
	assert.equal(f.runtime.errors.length, 1);
	assert.match(f.runtime.errors[0].error, /resource result limit/);
});

test("entry evaluation uses admitted bytes even if the pathname changes after capture", async (t) => {
	const f = fixture(t, [
		`export default api=>api.registerCommand('identity',{handler:()=> 'admitted'});`,
	]);
	const originalClose = fs.closeSync;
	let replaced = false;
	fs.closeSync = (fd) => {
		const filename = fs.readlinkSync(`/proc/self/fd/${fd}`);
		originalClose(fd);
		if (filename === f.entries[0].path && !replaced) {
			replaced = true;
			fs.writeFileSync(
				filename,
				`export default ()=>{throw new Error('unapproved replacement executed')};`,
			);
		}
	};
	let catalog;
	try {
		catalog = await f.runtime.load(f.entries);
	} finally {
		fs.closeSync = originalClose;
	}
	assert(replaced);
	assert.deepEqual(catalog.errors, []);
	await f.runtime.bind({});
	assert.equal(
		await f.runtime.invoke("identity", () => f.runtime.command("identity", "")),
		"admitted",
	);
	assert.match(
		fs.readFileSync(f.entries[0].path, "utf8"),
		/unapproved replacement/,
	);
});

test("captured TypeScript entries resolve their own node_modules and relative imports with top-level await", async (t) => {
	const f = fixture(t, [
		`import {message} from 'fixture-local-only';import {suffix} from './suffix.ts';const value=await Promise.resolve(message+suffix);export default api=>api.registerCommand('dependency',{handler:()=>value});`,
	]);
	const dependency = path.join(f.root, "node_modules/fixture-local-only");
	fs.mkdirSync(dependency, { recursive: true });
	fs.writeFileSync(
		path.join(dependency, "package.json"),
		JSON.stringify({
			name: "fixture-local-only",
			type: "module",
			exports: "./index.js",
		}),
	);
	fs.writeFileSync(
		path.join(dependency, "index.js"),
		`export const message='owned dependency';`,
	);
	fs.writeFileSync(
		path.join(f.root, "suffix.ts"),
		`export const suffix: string=' and relative TypeScript';`,
	);
	const catalog = await f.runtime.load(f.entries);
	assert.deepEqual(catalog.errors, []);
	await f.runtime.bind({});
	assert.equal(
		await f.runtime.invoke("dependency", () =>
			f.runtime.command("dependency", ""),
		),
		"owned dependency and relative TypeScript",
	);
});

test("abandoned complete promises settle through the native operation without an unhandled wrapper rejection", async (t) => {
	const f = fixture(
		t,
		[
			`import {complete} from '@ag1/ai';export default api=>{api.registerCommand('abandon',{handler:()=>{complete({provider:'fixture',id:'model'},{messages:[]});return 'abandoned';}});api.registerCommand('alive',{handler:()=> 'alive'});};`,
		],
		async (request) => {
			assert.equal(request.method, "inference.complete");
			throw new Error("native inference rejected");
		},
	);
	await f.runtime.load(f.entries);
	await f.runtime.bind({});
	await assert.rejects(
		f.runtime.invoke("abandon", () => f.runtime.command("abandon", "")),
		/native inference rejected/,
	);
	await new Promise((resolve) => setImmediate(resolve));
	assert.equal(f.runtime.pending.length, 0);
	assert.equal(
		await f.runtime.invoke("alive", () => f.runtime.command("alive", "")),
		"alive",
	);
});

test("selected loader registrations preserve legacy flags and isolate syntax, factory and missing-default failures", async (t) => {
	const f = fixture(t, [
		`export default api=>{api.registerTool({name:'tool-a',description:'A',parameters:{type:'object'},execute:async()=>({content:[]})});api.registerFlag('legacy',{description:'Legacy flag',handler:async()=>{throw new Error('metadata must not execute')}});api.on('agent_start',()=>{});api.on('agent_end',()=>{});api.on('tool_call',()=>{});api.registerShortcut('ctrl+t',{description:'Test shortcut',handler:async()=> 'shortcut'});api.registerMessageRenderer('custom',()=>null);};`,
		`this is not valid typescript export`,
		`export default ()=>{throw new Error('Initialization failed!')};`,
		`export function notDefault(){}`,
		`export default api=>{api.registerTool({name:'tool-b',description:'B',parameters:{type:'object'},execute:async()=>({content:[]} )});api.registerCommand('test',{handler:()=> 'registered'});};`,
	]);
	const catalog = await f.runtime.load(f.entries);
	assert.equal(catalog.extensions.length, 2);
	assert.deepEqual(
		catalog.tools.map((tool) => tool.name),
		["tool-a", "tool-b"],
	);
	assert.equal(catalog.errors.length, 3);
	assert.deepEqual(
		catalog.errors.map((error) => error.extensionPath),
		f.entries.slice(1, 4).map((entry) => entry.path),
	);
	assert.match(catalog.errors[1].error, /Initialization failed!/);
	assert.match(
		catalog.errors[2].error,
		/does not export a valid factory function/,
	);
	assert.deepEqual(catalog.flags, [
		{
			name: "legacy",
			description: "Legacy flag",
			extensionPath: f.entries[0].path,
		},
	]);
	assert.equal(
		typeof f.runtime.extensions[0].flags.get("legacy").handler,
		"function",
	);
	assert.deepEqual(catalog.handlers, ["agent_start", "agent_end", "tool_call"]);
	assert.equal(catalog.shortcuts[0].shortcut, "ctrl+t");
	assert.deepEqual(catalog.renderers, [
		{ type: "custom", extensionPath: f.entries[0].path },
	]);
	assert.equal(
		f.runtime.extensions[0].renderers.get("custom")({}, {}, {}),
		null,
	);
	await f.runtime.bind({});
	assert.equal(
		await f.runtime.invoke("test", () => f.runtime.command("test", "")),
		"registered",
	);
});

test("input hooks preserve all source types and replace image payloads without changing attribution", async (t) => {
	const f = fixture(t, [
		`export default api=>{let source;api.on('input',e=>{source=e.source;return {action:'transform',text:e.text,images:[{type:'image',data:'new',mimeType:'image/jpeg'}]}});api.registerCommand('source',{handler:()=>source});};`,
	]);
	await f.runtime.load(f.entries);
	await f.runtime.bind({});
	for (const source of ["interactive", "rpc", "extension"]) {
		const result = await f.runtime.invoke(`input-${source}`, () =>
			f.runtime.event({
				type: "input",
				text: "hi",
				images: [{ type: "image", data: "orig", mimeType: "image/png" }],
				source,
			}),
		);
		assert.deepEqual(result, {
			action: "transform",
			text: "hi",
			images: [{ type: "image", data: "new", mimeType: "image/jpeg" }],
		});
		assert.equal(
			await f.runtime.invoke(`source-${source}`, () =>
				f.runtime.command("source", ""),
			),
			source,
		);
	}
});

test("optional native host requests preserve arguments and context hook failures reach the error listener", async (t) => {
	const calls = [];
	const f = fixture(
		t,
		[
			`export default api=>{api.registerCommand('host',{handler:async(_,ctx)=>ctx.host?ctx.host.request('godot.describe',{type:'Node3D'}):'absent'});api.on('context',()=>{throw new Error('Handler error!')});};`,
		],
		async (call) => {
			calls.push(call);
			assert.equal(call.method, "host.request");
			assert.deepEqual(call.args, {
				method: "godot.describe",
				params: { type: "Node3D" },
			});
			return { selected: "Node3D" };
		},
	);
	await f.runtime.load(f.entries);
	await f.runtime.bind({ hasHost: false });
	assert.equal(
		await f.runtime.invoke("absent", () => f.runtime.command("host", "")),
		"absent",
	);
	assert.equal(calls.length, 0);
	assert.deepEqual(
		await f.runtime.invoke("host", () => f.runtime.command("host", ""), {
			hasHost: true,
		}),
		{ selected: "Node3D" },
	);
	assert.deepEqual(
		await f.runtime.invoke("context", () =>
			f.runtime.event({ type: "context", messages: [] }),
		),
		[],
	);
	assert.equal(f.events.length, 1);
	assert.equal(f.events[0].type, "extension_error");
	assert.equal(f.events[0].event, "context");
	assert.equal(f.events[0].error, "Handler error!");
});

test("shortcut reservations follow current bindings including freed defaults and multiple nonreserved keys", async (t) => {
	const f = fixture(t, [
		`export default api=>{for(const key of ['ctrl+p','ctrl+x','ctrl+y'])api.registerShortcut(key,{handler:()=>key})};`,
	]);
	await f.runtime.load(f.entries);
	await f.runtime.bind({});
	let result = f.runtime.shortcuts({ "app.model.cycleForward": "ctrl+n" });
	assert(result.shortcuts.some((s) => s.key === "ctrl+p"));
	assert.equal(result.diagnostics.length, 0);
	result = f.runtime.shortcuts({ "app.interrupt": ["ctrl+x", "ctrl+y"] });
	assert.deepEqual(
		result.shortcuts.map((s) => s.key),
		["ctrl+p"],
	);
	assert.equal(result.diagnostics.length, 2);
	assert(
		result.diagnostics.every((d) =>
			d.message.includes("conflicts with built-in"),
		),
	);
	result = f.runtime.shortcuts({
		"app.clipboard.pasteImage": ["ctrl+x", "ctrl+y"],
	});
	assert.deepEqual(
		result.shortcuts.map((s) => s.key),
		["ctrl+p", "ctrl+x", "ctrl+y"],
	);
	assert.equal(result.diagnostics.length, 2);
	assert(
		result.diagnostics.every((d) =>
			d.message.includes("built-in shortcut for app.clipboard.pasteImage"),
		),
	);
	assert.equal(
		await f.runtime.invoke("shortcut", () =>
			f.runtime.shortcut("ctrl+y", {
				"app.clipboard.pasteImage": ["ctrl+x", "ctrl+y"],
			}),
		),
		"ctrl+y",
	);
});

test("resource loader conflicts retain both commands and report the SDK tool and flag winners", async (t) => {
	const oracle = JSON.parse(
		fs.readFileSync(
			new URL(
				"../../../../fixtures/tools/loader-selection-oracle.json",
				import.meta.url,
			),
			"utf8",
		),
	);
	let checked = 0;
	for (const row of oracle.rows.filter((row) => row.output.extensions.length)) {
		const owned = fs.mkdtempSync(
			path.join(os.tmpdir(), "ag1-resource-conflicts-"),
		);
		t.after(() => fs.rmSync(owned, { recursive: true, force: true }));
		for (const [p, code] of Object.entries(row.input.files)) {
			fs.mkdirSync(path.dirname(path.join(owned, p)), { recursive: true });
			fs.writeFileSync(path.join(owned, p), code);
		}
		const entries = row.output.extensions.map((ext) => {
			const target = ext.path.replace("/fixture", owned),
				code = fs.readFileSync(target);
			return {
				path: target,
				sha256: createHash("sha256").update(code).digest("hex"),
				sourceInfo: {
					path: target,
					source: "fixture",
					scope: "temporary",
					origin: "top-level",
				},
			};
		});
		const runtime = new ExtensionRuntime(async () => {}, { cwd: owned });
		const catalog = await runtime.load(entries);
		const normalize = (value) =>
			JSON.parse(JSON.stringify(value).replaceAll(owned, "/fixture"));
		assert.deepEqual(catalog.errors, []);
		assert.deepEqual(normalize(catalog.conflicts), row.output.errors);
		assert.equal(
			catalog.tools[0].description,
			row.output.extensions[0].tools[0].description,
		);
		assert.deepEqual(
			catalog.commands.map((c) => c.invocationName),
			["deploy:1", "deploy:2"],
		);
		assert.deepEqual(
			catalog.commands.map((c) => c.description),
			row.output.extensions.map((e) => e.commands[0].description),
		);
		await runtime.bind({});
		for (const [i, ext] of row.output.extensions.entries()) {
			assert.equal(
				await runtime.invoke("command", () =>
					runtime.command(`deploy:${i + 1}`, ""),
				),
				ext.commands[0].description.replace(" command", ""),
			);
		}
		assert.deepEqual(normalize(runtime.catalog().conflicts), row.output.errors);
		assert.equal(catalog.extensions.length, 2);
		checked++;
	}
	assert.equal(checked, 2);
});
