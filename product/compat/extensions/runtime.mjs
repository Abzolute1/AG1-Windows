// Optional trusted-code runtime. Parent state, policy and inference are RPC ports.
// No AgentSession, file/session store, model client or controller is imported here.
import fs from "node:fs";
import path from "node:path";
import { createHash } from "node:crypto";
import { AsyncLocalStorage } from "node:async_hooks";
import { createJiti } from "./vendor/node_modules/@mariozechner/jiti/lib/jiti.mjs";
import {
	AssistantMessageEventStream,
	createAssistantMessageEventStream,
} from "./event-stream.mjs";
import { resolveShortcuts } from "./shortcuts.mjs";
import * as TypeBox from "./vendor/node_modules/@sinclair/typebox/build/esm/index.mjs";

export class UnsupportedAPI extends Error {
	constructor(name) {
		super(`Unsupported extension API: ${name}`);
		this.code = "unsupported_api";
	}
}
export function bounded(value, limit = 1024 * 1024, maxNodes = 100000) {
	const pending = [[value, 0]];
	let count = 0;
	while (pending.length) {
		const [v, depth] = pending.pop();
		if (++count > maxNodes || depth > 48)
			throw new Error("Extension value exceeds structural bounds");
		if (typeof v === "number" && !Number.isFinite(v))
			throw new Error("Extension protocol rejects non-finite numbers");
		if (v && typeof v === "object") {
			const entries = Object.values(v);
			if (pending.length + entries.length + count > maxNodes)
				throw new Error("Extension value exceeds structural bounds");
			for (const child of entries) pending.push([child, depth + 1]);
		} else if (
			typeof v === "function" ||
			typeof v === "symbol" ||
			typeof v === "bigint"
		) {
			throw new Error("Extension protocol accepts data only");
		}
	}
	const text = JSON.stringify(value);
	if (text !== undefined && Buffer.byteLength(text) > limit)
		throw new Error("Extension value exceeds byte limit");
	return value;
}
const clone = (v) =>
	v === undefined ? undefined : structuredClone(bounded(v));
// Native inspection snapshots have a separate bound; callback results and
// native action arguments retain the ordinary one-frame limit.
const cloneSnapshot = (v) => v === undefined ? undefined :
	structuredClone(bounded(v, 8 * 1024 * 1024, 250000));
function name(value) {
	if (
		typeof value !== "string" ||
		!value ||
		value.length > 256 ||
		value.includes("\0")
	)
		throw new Error("Invalid extension registration name");
	return value;
}
function ownMap(map, key, value, limit = 256) {
	if (!map.has(key) && map.size >= limit)
		throw new Error("Extension registration limit");
	map.set(key, value);
}
function callable(value, label) {
	if (typeof value !== "function") throw new Error(`Invalid ${label} handler`);
	return value;
}
function unsupported(name) {
	return () => {
		throw new UnsupportedAPI(name);
	};
}
function dataDefinition(value) {
	const result = {};
	for (const [key, item] of Object.entries(value))
		if (typeof item !== "function") result[key] = item;
	// TypeBox schemas contain symbol metadata; ordinary JSON deliberately excludes
	// those symbols, just as the SDK's model schema serialization does.
	return bounded(JSON.parse(JSON.stringify(result)));
}
function immutable(value) {
	if (value && typeof value === "object") {
		for (const child of Object.values(value)) immutable(child);
		Object.freeze(value);
	}
	return value;
}

/** The embedding process supplies the broker. A callback is never parent authority. */
export class ExtensionRuntime {
	constructor(bridge, { cwd, emit = () => {}, bridgeSync } = {}) {
		callable(bridge, "native bridge");
		if (typeof cwd !== "string" || !path.isAbsolute(cwd))
			throw new Error("Absolute extension cwd required");
		this.bridge = bridge;
		this.bridgeSync = bridgeSync;
		this.cwd = cwd;
		this.emit = emit;
		this.extensions = [];
		this.flags = new Map();
		this.providers = new Map();
		this.providerVersions = new Map();
		this.providerSerial = 0;
		this.invocations = new AsyncLocalStorage();
		this.pendingProviders = [];
		this.pending = [];
		this.errors = [];
		this.bound = false;
		this.current = undefined;
		this.snapshot = {};
		this.events = new Map();
		this.generation = 0;
		this.registrationRevision = 0;
	}
	changed() {
		if (this.registrationRevision >= Number.MAX_SAFE_INTEGER)
			throw new Error("Extension registration revision exhausted");
		this.registrationRevision++;
	}

	error(extensionPath, event, error) {
		const item = {
			extensionPath,
			event,
			error: String(error?.message ?? error).slice(0, 8192),
		};
		if (this.errors.length >= 256) this.errors.shift();
		this.errors.push(item);
		this.emit({ type: "extension_error", ...item });
	}
	async request(extension, method, args, onEvent, signal) {
		if (!this.bound)
			throw new Error(
				"Extension runtime not initialized. Action methods cannot be called during extension loading.",
			);
		if (!this.current)
			throw new Error("Extension action requires an active native operation");
		if (this.current.signal.aborted)
			throw new Error("Extension operation cancelled");
		return clone(
			await this.bridge(
				{
					extension: extension.path,
					generation: this.generation,
					operation: this.current.id,
					invocation: this.invocations.getStore(),
					method,
					args: clone(args),
				},
				onEvent,
				signal ?? this.current.signal,
			),
		);
	}
	requestSync(extension, method, args) {
		if (!this.bound || !this.current)
			throw new Error("Extension action requires an active native operation");
		if (this.current.signal.aborted)
			throw new Error("Extension operation cancelled");
		if (!this.bridgeSync) throw new UnsupportedAPI("synchronous native bridge");
		return clone(
			this.bridgeSync(
				{
					extension: extension.path,
					generation: this.generation,
					operation: this.current.id,
					invocation: this.invocations.getStore(),
					method,
					args: clone(args),
				},
				this.current.signal,
			),
		);
	}
	enqueue(extension, method, args) {
		// Void SDK actions still need native acknowledgement before operation result.
		if (!this.bound || !this.current)
			throw new Error(
				"Extension runtime not initialized. Action methods cannot be called during extension loading.",
			);
		if (this.pending.length >= 256)
			throw new Error("Extension pending action limit");
		const promise = this.request(extension, method, args);
		promise.catch(() => {}); // flush propagates the failure without unhandled rejection.
		this.pending.push(promise);
	}
	async flush() {
		const pending = this.pending;
		this.pending = [];
		const results = await Promise.allSettled(pending);
		const failed = results.find((r) => r.status === "rejected");
		if (failed) throw failed.reason;
	}
	async load(entries) {
		if (this.current || this.bound || this.extensions.length)
			throw new Error("Reload requires a fresh owned extension host");
		bounded(entries);
		if (!Array.isArray(entries) || entries.length > 64)
			throw new Error("Extension entry limit");
		for (const entry of entries) {
			if (
				typeof entry.path !== "string" ||
				!path.isAbsolute(entry.path) ||
				!/^[a-f0-9]{64}$/.test(entry.sha256)
			)
				throw new Error("Invalid admitted extension entry");
			// This validates the granted entry bytes. It is not a sandbox for imported
			// dependencies or trusted code's own filesystem/network operations.
			const fd = fs.openSync(entry.path, "r");
			let bytes;
			try {
				if (fs.fstatSync(fd).size > 1024 * 1024)
					throw new Error("Extension entry byte limit");
				const buffer = Buffer.alloc(1024 * 1024 + 1);
				let count = 0;
				while (count < buffer.length) {
					const n = fs.readSync(fd, buffer, count, buffer.length - count, null);
					if (!n) break;
					count += n;
				}
				bytes = buffer.subarray(0, count);
			} finally {
				fs.closeSync(fd);
			}
			if (
				bytes.length > 1024 * 1024 ||
				createHash("sha256").update(bytes).digest("hex") !== entry.sha256
			)
				throw new Error("Stale extension entry");
			const extension = {
				path: entry.path,
				sourceInfo: clone(entry.sourceInfo),
				handlers: new Map(),
				tools: new Map(),
				commands: new Map(),
				shortcuts: new Map(),
				flags: new Map(),
				renderers: new Map(),
			};
			const coding = {
				defineTool: (tool) => tool,
				createSyntheticSourceInfo: (p, options = {}) => ({
					path: p,
					source: options.source ?? "temporary",
					scope: "temporary",
					origin: "top-level",
					...(options.baseDir ? { baseDir: options.baseDir } : {}),
				}),
			};
			const ai = this.aiModule(extension);
			const modules = {
				"@sinclair/typebox": TypeBox,
				"@ag1/coding-agent": coding,
				"@ag1/ai": ai,
				"@ag1/ai/oauth": new Proxy(
					{},
					{ get: (_, key) => unsupported(`ai/oauth.${String(key)}`) },
				),
				"@ag1/agent-core": new Proxy(
					{},
					{ get: (_, key) => unsupported(`agent-core.${String(key)}`) },
				),
				"@ag1/tui": this.tuiModule(),
			};
			const jiti = createJiti(import.meta.url, {
				moduleCache: false,
				fsCache: false,
				tryNative: false,
				virtualModules: modules,
			});
			try {
				// Evaluate the bytes whose digest was admitted. Reopening the filename
				// here would allow a replacement between verification and evaluation.
				// The filename still anchors relative imports and local node_modules.
				const loaded = await jiti.evalModule(bytes.toString("utf8"), {
					filename: entry.path,
					async: true,
				});
				const factory = loaded?.default ?? loaded;
				if (typeof factory !== "function")
					throw new Error(
						`Extension does not export a valid factory function: ${entry.path}`,
					);
				await factory(this.api(extension));
				this.extensions.push(extension);
			} catch (error) {
				this.error(entry.path, "load", error);
			}
		}
		return this.catalog();
	}
	async bind(snapshot, operation = "bind") {
		if (this.bound) throw new Error("Extension runtime already bound");
		this.snapshot = immutable(cloneSnapshot(snapshot));
		this.bound = true;
		return this.invoke(operation, async () => {
			const registrations = this.pendingProviders;
			this.pendingProviders = [];
			for (const registration of registrations) {
				try {
					await this.registerProviderNow(...registration);
				} catch (error) {
					this.error(registration[0].path, "register_provider", error);
				}
			}
			return this.catalog();
		});
	}
	async invoke(id, operation, snapshot) {
		if (this.current)
			throw new Error("Extension host operation already active");
		name(id);
		if (snapshot !== undefined) this.snapshot = immutable(cloneSnapshot(snapshot));
		const controller = new AbortController();
		this.current = { id, controller, signal: controller.signal };
		try {
			let result;
			try {
				result = await operation();
			} finally {
				await this.flush();
			}
			return clone(result);
		} finally {
			controller.abort();
			this.current = undefined;
		}
	}
	cancel(id) {
		if (this.current?.id === id) this.current.controller.abort();
	}
	api(extension) {
		const runtime = this;
		const api = {
			on(event, handler) {
				name(event);
				callable(handler, "event");
				const handlers = extension.handlers.get(event) ?? [];
				if (handlers.length >= 64) throw new Error("Extension handler limit");
				handlers.push(handler);
				ownMap(extension.handlers, event, handlers);
				runtime.changed();
			},
			registerTool(tool) {
				name(tool?.name);
				callable(tool.execute, "tool");
				const definition = dataDefinition(tool);
				ownMap(extension.tools, tool.name, {
					definition,
					execute: tool.execute,
					prepareArguments: tool.prepareArguments,
					renderCall: tool.renderCall,
					renderResult: tool.renderResult,
				});
				runtime.changed();
				if (runtime.bound)
					runtime.enqueue(extension, "registry.refreshTools", {
						tools: runtime.catalog().tools,
					});
			},
			registerCommand(command, options) {
				name(command);
				callable(options?.handler, "command");
				ownMap(extension.commands, command, { ...options, name: command });
				runtime.changed();
			},
			registerShortcut(shortcut, options) {
				name(shortcut);
				callable(options?.handler, "shortcut");
				ownMap(extension.shortcuts, shortcut, { ...options, shortcut });
				runtime.changed();
			},
			registerFlag(flag, options) {
				name(flag);
				if (!options || typeof options !== "object" || Array.isArray(options))
					throw new Error("Invalid extension flag");
				// The pinned loader accepts legacy flags without a declared type,
				// including function-valued metadata. Functions remain helper-local.
				dataDefinition(options);
				ownMap(extension.flags, flag, { name: flag, ...options });
				runtime.changed();
				if (options.default !== undefined && !runtime.flags.has(flag))
					ownMap(runtime.flags, flag, clone(options.default));
			},
			registerMessageRenderer(type, renderer) {
				name(type);
				callable(renderer, "message renderer");
				ownMap(extension.renderers, type, renderer);
				runtime.changed();
			},
			getFlag: (flag) =>
				extension.flags.has(flag) ? runtime.flags.get(flag) : undefined,
			registerProvider(provider, config) {
				name(provider);
				if (!runtime.bound) {
					if (runtime.pendingProviders.length >= 256)
						throw new Error("Provider registration limit");
					runtime.pendingProviders.push([extension, provider, config]);
				} else if (runtime.bridgeSync) {
					runtime.registerProviderSync(extension, provider, config);
				} else {
					if (runtime.pending.length >= 256)
						throw new Error("Extension pending action limit");
					const promise = runtime.registerProviderNow(
						extension,
						provider,
						config,
					);
					promise.catch(() => {});
					runtime.pending.push(promise);
				}
			},
			unregisterProvider(provider) {
				if (!runtime.bound)
					runtime.pendingProviders = runtime.pendingProviders.filter(
						(r) => r[1] !== provider,
					);
				else if (runtime.bridgeSync) {
					const answer = runtime.requestSync(extension, "provider.unregister", {
						name: provider,
					});
					runtime.providers.delete(provider);
					runtime.retainProviderVersions(answer?.retainRegistrations);
					runtime.updateProviderModels(answer);
				} else {
					if (runtime.pending.length >= 256)
						throw new Error("Extension pending action limit");
					const promise = runtime
						.request(extension, "provider.unregister", { name: provider })
						.then((answer) => {
							runtime.providers.delete(provider);
							runtime.retainProviderVersions(answer?.retainRegistrations);
							runtime.updateProviderModels(answer);
						});
					promise.catch(() => {});
					runtime.pending.push(promise);
				}
			},
			getSessionName: () => runtime.read("sessionName"),
			getActiveTools: () => clone(runtime.read("activeTools") ?? []),
			getAllTools: () => clone(runtime.read("allTools") ?? []),
			getCommands: () =>
				clone(runtime.read("commands") ?? runtime.catalog().commands),
			getThinkingLevel: () => runtime.read("thinkingLevel"),
			setModel: (model) => runtime.request(extension, "model.set", { model }),
			exec: (command, args, options) =>
				runtime.request(extension, "process.exec", { command, args, options }),
			events: {
				on(event, handler) {
					name(event);
					callable(handler, "event bus");
					const list = runtime.events.get(event) ?? new Set();
					if (list.size >= 64) throw new Error("Event listener limit");
					list.add(handler);
					ownMap(runtime.events, event, list);
					return () => list.delete(handler);
				},
				off(event, handler) {
					runtime.events.get(event)?.delete(handler);
				},
				emit(event, data) {
					bounded(data);
					for (const handler of runtime.events.get(event) ?? [])
						handler(clone(data));
				},
			},
		};
		for (const method of [
			"sendMessage",
			"sendUserMessage",
			"appendEntry",
			"setSessionName",
			"setLabel",
			"setActiveTools",
			"setThinkingLevel",
		])
			api[method] = (...args) =>
				runtime.enqueue(extension, `session.${method}`, { args });
		return new Proxy(api, {
			get: (target, key) =>
				key in target
					? target[key]
					: unsupported(`ExtensionAPI.${String(key)}`),
		});
	}
	read(key) {
		if (!this.bound)
			throw new Error(
				"Extension runtime not initialized. Action methods cannot be called during extension loading.",
			);
		return this.snapshot[key];
	}
	context(extension, command = false) {
		const request = (method, args) => this.request(extension, method, args);
		const queue = (method, args) => this.enqueue(extension, method, args);
		const ui = {};
		for (const method of ["select", "confirm", "input", "editor"])
			ui[method] = (...args) => request(`ui.${method}`, { args });
		for (const method of [
			"notify",
			"setStatus",
			"setWorkingMessage",
			"setHiddenThinkingLabel",
			"setTitle",
			"pasteToEditor",
			"setEditorText",
			"setToolsExpanded",
		])
			ui[method] = (...args) => queue(`ui.${method}`, { args });
		ui.setWidget = (key, content, options) => {
			if (typeof content === "function")
				throw new UnsupportedAPI("ui.setWidget component factory");
			queue("ui.setWidget", { key, content, options });
		};
		for (const method of [
			"onTerminalInput",
			"setFooter",
			"setHeader",
			"custom",
			"setEditorComponent",
		])
			ui[method] = unsupported(`ui.${method}`);
		ui.getEditorText = () => this.snapshot.ui?.editorText ?? "";
		ui.getToolsExpanded = () => this.snapshot.ui?.toolsExpanded ?? false;
		ui.getAllThemes = () => clone(this.snapshot.ui?.themes ?? []);
		ui.getTheme = (name) =>
			clone(this.snapshot.ui?.themes?.find((t) => t.name === name));
		ui.setTheme = unsupported("ui.setTheme synchronous native acknowledgement");
		ui.theme = this.tuiModule().theme;
		const session = new Proxy(
			{},
			{
				get: (_, method) => {
					if (
						[
							"getSessionId",
							"getSessionFile",
							"getLeafId",
							"getEntries",
							"getBranch",
							"getTree",
							"getHeader",
							"getLabel",
							"getEntry",
							"getSessionName",
							"getCwd",
						].includes(method)
					)
						return (...args) => {
							const s = this.snapshot.session ?? {};
							const entries = s.entries ?? [];
							if (method === "getEntry")
								return cloneSnapshot(entries.find((e) => e.id === args[0]));
							if (method === "getLabel") return s.labels?.[args[0]];
							if (method === "getBranch" && args[0]) {
								const byId = new Map(entries.map((entry) => [entry.id, entry]));
								const branch = [], seen = new Set();
								let entry = byId.get(args[0]);
								while (entry) {
									if (seen.has(entry.id)) throw new Error("Cyclic session branch");
									seen.add(entry.id);
									branch.push(entry);
									entry = byId.get(entry.parentId);
								}
								return cloneSnapshot(branch.reverse());
							}
							if (method === "getTree" && s.tree === undefined) {
								const nodes = new Map(cloneSnapshot(entries).map((entry) => [entry.id, {
									entry, children: [], label: s.labels?.[entry.id], labelTimestamp: s.labelTimestamps?.[entry.id],
								}]));
								const roots = [];
								for (const node of nodes.values()) {
									const parent = nodes.get(node.entry.parentId);
									if (parent && parent !== node) parent.children.push(node);
									else roots.push(node);
								}
								for (const node of nodes.values()) node.children.sort((a,b) => new Date(a.entry.timestamp) - new Date(b.entry.timestamp));
								return roots;
							}
							const field = {
								getSessionId: "id",
								getSessionFile: "file",
								getLeafId: "leafId",
								getEntries: "entries",
								getBranch: "branch",
								getTree: "tree",
								getHeader: "header",
								getSessionName: "name",
								getCwd: "cwd",
							}[method];
							return cloneSnapshot(s[field]);
						};
					return unsupported(`ReadonlySessionManager.${String(method)}`);
				},
			},
		);
		const ctx = {
			ui: new Proxy(ui, {
				get: (target, key) =>
					key in target ? target[key] : unsupported(`ui.${String(key)}`),
			}),
			hasUI: !!this.snapshot.hasUI,
			cwd: this.cwd,
			sessionManager: session,
			model: clone(this.snapshot.model),
			signal: this.current?.signal,
			modelRegistry: {
				getAll: () => clone(this.snapshot.models ?? []),
				getAvailable: () => clone(this.snapshot.availableModels ?? []),
				find: (provider, id) =>
					clone(
						this.snapshot.models?.find(
							(m) => m.provider === provider && m.id === id,
						),
					),
				getApiKey: (model) => request("auth.getApiKey", { model }),
				getApiKeyForProvider: (provider) =>
					request("auth.getApiKeyForProvider", { provider }),
			},
			isIdle: () => !!this.snapshot.isIdle,
			hasPendingMessages: () => !!this.snapshot.hasPendingMessages,
			getContextUsage: () => clone(this.snapshot.contextUsage),
			getSystemPrompt: () => this.snapshot.systemPrompt ?? "",
			abort: () => queue("session.abort", {}),
			shutdown: () => queue("session.shutdown", {}),
			compact: (options) => {
				if (options?.onComplete || options?.onError)
					throw new UnsupportedAPI("compact callbacks");
				queue("session.compact", { options });
			},
		};
		if (this.snapshot.hasHost)
			ctx.host = {
				request: (method, params) =>
					request("host.request", { method, params }),
			};
		if (command)
			for (const method of [
				"waitForIdle",
				"newSession",
				"fork",
				"navigateTree",
				"switchSession",
				"reload",
			])
				ctx[method] = (...args) => request(`session.${method}`, { args });
		return ctx;
	}
	prepareProvider(extension, provider, config) {
		const definition = dataDefinition(config);
		definition.hooks = {
			streamSimple: typeof config.streamSimple === "function",
			oauth: config.oauth
				? Object.fromEntries(
						["login", "refreshToken", "getApiKey", "modifyModels"].map((k) => [
							k,
							typeof config.oauth[k] === "function",
						]),
					)
				: undefined,
		};
		if (config.oauth) definition.oauth = dataDefinition(config.oauth);
		if (
			this.providerVersions.size >= 256 ||
			(!this.providers.has(provider) && this.providers.size >= 256)
		)
			throw new Error("Provider hook version limit");
		if (this.providerSerial >= Number.MAX_SAFE_INTEGER)
			throw new Error("Provider hook version exhausted");
		const registration = `provider-hook-${++this.providerSerial}`;
		const version = {
			extension,
			config: {
				...config,
				oauth: config.oauth ? { ...config.oauth } : undefined,
			},
			provider,
			registration,
			pending: true,
		};
		// The native owner may invoke a proposed OAuth hook while validating this
		// registration. A rejected proposal is removed without replacing the old one.
		this.providerVersions.set(registration, version);
		return {
			version,
			args: { name: provider, config: definition, registration },
		};
	}
	acceptProvider(version, answer) {
		version.pending = false;
		if (
			answer?.retainRegistrations !== undefined &&
			!answer.retainRegistrations.includes(version.registration)
		)
			throw new Error(
				"Native provider acknowledgement omitted its registration",
			);
		this.retainProviderVersions(answer?.retainRegistrations);
		this.updateProviderModels(answer);
		ownMap(this.providers, version.provider, version);
	}
	async registerProviderNow(extension, provider, config) {
		const { version, args } = this.prepareProvider(extension, provider, config);
		try {
			this.acceptProvider(
				version,
				await this.request(extension, "provider.register", args),
			);
		} catch (error) {
			this.providerVersions.delete(version.registration);
			throw error;
		}
	}
	registerProviderSync(extension, provider, config) {
		const { version, args } = this.prepareProvider(extension, provider, config);
		try {
			this.acceptProvider(
				version,
				this.requestSync(extension, "provider.register", args),
			);
		} catch (error) {
			this.providerVersions.delete(version.registration);
			throw error;
		}
	}
	retainProviderVersions(registrations) {
		if (registrations === undefined) return;
		bounded(registrations);
		if (
			!Array.isArray(registrations) ||
			registrations.length > 256 ||
			registrations.some((value) => typeof value !== "string")
		)
			throw new Error("Invalid native provider retention set");
		const retained = new Set(registrations);
		for (const [key, version] of this.providerVersions)
			if (!version.pending && !retained.has(key))
				this.providerVersions.delete(key);
	}
	updateProviderModels(answer) {
		if (answer?.models === undefined) return;
		if (!Array.isArray(answer.models))
			throw new Error("Invalid native model catalog");
		this.snapshot = immutable({
			...this.snapshot,
			models: clone(answer.models),
		});
	}
	catalog() {
		const tools = [],
			commands = [],
			flags = [],
			conflicts = [],
			shortcuts = [],
			renderers = [],
			handlers = new Set();
		const toolNames = new Map(),
			flagNames = new Map(),
			commandCounts = new Map(),
			commandSeen = new Map(),
			taken = new Set();
		for (const ext of this.extensions)
			for (const command of ext.commands.keys())
				commandCounts.set(command, (commandCounts.get(command) ?? 0) + 1);
		for (const ext of this.extensions) {
			for (const [tool, value] of ext.tools) {
				if (toolNames.has(tool)) {
					conflicts.push({
						path: ext.path,
						error: `Tool "${tool}" conflicts with ${toolNames.get(tool)}`,
					});
					continue;
				}
				toolNames.set(tool, ext.path);
				tools.push({
					...value.definition,
					extensionPath: ext.path,
					sourceInfo: ext.sourceInfo,
					hasPrepareArguments: !!value.prepareArguments,
					hasRenderCall: !!value.renderCall,
					hasRenderResult: !!value.renderResult,
				});
			}
			for (const [command, value] of ext.commands) {
				const n = (commandSeen.get(command) ?? 0) + 1;
				commandSeen.set(command, n);
				let invocationName =
					commandCounts.get(command) > 1 ? `${command}:${n}` : command;
				let suffix = n;
				while (taken.has(invocationName))
					invocationName = `${command}:${++suffix}`;
				taken.add(invocationName);
				commands.push({
					...dataDefinition(value),
					name: command,
					invocationName,
					extensionPath: ext.path,
					sourceInfo: ext.sourceInfo,
				});
			}
			for (const [flag, value] of ext.flags) {
				if (flagNames.has(flag)) {
					conflicts.push({
						path: ext.path,
						error: `Flag "--${flag}" conflicts with ${flagNames.get(flag)}`,
					});
					continue;
				}
				flagNames.set(flag, ext.path);
				flags.push({ ...dataDefinition(value), extensionPath: ext.path });
			}
			for (const value of ext.shortcuts.values())
				shortcuts.push({ ...dataDefinition(value), extensionPath: ext.path });
			for (const type of ext.renderers.keys())
				renderers.push({ type, extensionPath: ext.path });
			for (const type of ext.handlers.keys()) handlers.add(type);
		}
		return bounded({
			generation: this.generation,
			registrationRevision: this.registrationRevision,
			extensions: this.extensions.map((e) => ({
				path: e.path,
				sourceInfo: e.sourceInfo,
			})),
			tools,
			commands,
			flags,
			shortcuts,
			renderers,
			handlers: [...handlers],
			errors: this.errors,
			conflicts,
		});
	}
	setFlagValue(flag, value) {
		name(flag);
		if (typeof value !== "boolean" && typeof value !== "string")
			throw new Error("Invalid flag value");
		bounded(value);
		ownMap(this.flags, flag, value);
	}
	getFlagValues() {
		return clone(Object.fromEntries(this.flags));
	}
	shortcuts(keybindings) {
		bounded(keybindings);
		const { shortcuts, diagnostics } = resolveShortcuts(
			this.extensions,
			keybindings,
		);
		return bounded({
			shortcuts: [...shortcuts].map(([key, { extension, shortcut }]) => ({
				key,
				...dataDefinition(shortcut),
				extensionPath: extension.path,
			})),
			diagnostics,
		});
	}
	async shortcut(key, keybindings) {
		name(key);
		bounded(keybindings);
		const { shortcuts } = resolveShortcuts(this.extensions, keybindings);
		const selected = shortcuts.get(key.toLowerCase());
		if (!selected)
			throw new Error(`Unknown or reserved extension shortcut: ${key}`);
		return selected.shortcut.handler(this.context(selected.extension));
	}

	tool(name) {
		for (const extension of this.extensions) {
			const tool = extension.tools.get(name);
			if (tool) return { extension, tool };
		}
		throw new Error(`Unknown extension tool: ${name}`);
	}
	prepareTool(name, args) {
		const { tool } = this.tool(name);
		return clone(
			tool.prepareArguments ? tool.prepareArguments(clone(args)) : args,
		);
	}
	async executeTool(name, id, args) {
		const { extension, tool } = this.tool(name);
		return tool.execute(
			id,
			clone(args),
			this.current?.signal,
			(update) =>
				this.emit({
					type: "tool_update",
					operation: this.current?.id,
					tool: name,
					value: clone(update),
				}),
			this.context(extension),
		);
	}
	async command(name, args) {
		const command = this.catalog().commands.find(
			(c) => c.invocationName === name,
		);
		if (!command) throw new Error(`Unknown extension command: ${name}`);
		const ext = this.extensions.find((e) => e.path === command.extensionPath);
		return ext.commands
			.get(command.name)
			.handler(args, this.context(ext, true));
	}
	async event(event) {
		bounded(event);
		let current = clone(event),
			modified = false,
			last;
		const resources = { skillPaths: [], promptPaths: [], themePaths: [] };
		const messages = [];
		const sessionBefore = [
			"session_before_switch",
			"session_before_fork",
			"session_before_compact",
			"session_before_tree",
		].includes(event.type);
		for (const ext of this.extensions)
			for (const handler of ext.handlers.get(event.type) ?? []) {
				try {
					const handlerEvent = { ...current };
					if (["session_before_compact", "session_before_tree"].includes(event.type)) handlerEvent.signal = this.current.signal;
					const result = await handler(handlerEvent, this.context(ext));
					if (result === undefined) continue;
					bounded(result);
					if (!result && event.type !== "before_provider_request") continue;
					if (event.type === "input") {
						if (result.action === "handled") return result;
						if (result.action === "transform") {
							current.text = result.text;
							if (result.images != null && result.images !== current.images) {
								modified = true;
								current.images = result.images;
							}
						}
					} else if (event.type === "tool_result") {
						for (const key of ["content", "details", "isError"])
							if (result[key] !== undefined) {
								current[key] = result[key];
								modified = true;
							}
					} else if (event.type === "resources_discover") {
						// Validate the whole handler contribution before retaining any paths.
						// An isolated handler error must not publish its partial result.
						for (const key of Object.keys(resources)) {
							const paths = result[key] ?? [];
							if (!Array.isArray(paths))
								throw new Error("Invalid extension resource paths");
							if (resources[key].length + paths.length > 256)
								throw new Error("Extension resource result limit");
						}
						for (const key of Object.keys(resources))
							for (const p of result[key] ?? []) {
								resources[key].push({ path: p, extensionPath: ext.path });
							}
					} else if (event.type === "context") {
						if (result.messages) current.messages = result.messages;
					} else if (event.type === "before_provider_request") {
						current.payload = result;
					} else if (event.type === "before_agent_start") {
						if (result.message) messages.push(result.message);
						if (result.systemPrompt !== undefined) {
							current.systemPrompt = result.systemPrompt;
							modified = true;
						}
					} else if (event.type === "user_bash") {
						return result;
					} else if (event.type === "tool_call" || sessionBefore) {
						last = result;
						if (
							(result.block && event.type === "tool_call") ||
							(sessionBefore && result.cancel)
						)
							return result;
					}
				} catch (error) {
					this.error(ext.path, event.type, error);
				}
			}
		if (event.type === "input")
			return modified || current.text !== event.text
				? { action: "transform", text: current.text, images: current.images }
				: { action: "continue" };
		if (event.type === "tool_result")
			return modified
				? {
						content: current.content,
						details: current.details,
						isError: current.isError,
					}
				: undefined;
		if (event.type === "resources_discover") return resources;
		if (event.type === "context") return current.messages;
		if (event.type === "before_provider_request") return current.payload;
		if (event.type === "before_agent_start")
			return messages.length || modified
				? {
						messages: messages.length ? messages : undefined,
						systemPrompt: modified ? current.systemPrompt : undefined,
					}
				: undefined;
		return last;
	}
	providerSync(name, hook, args, registration) {
		const registered = this.providerVersions.get(registration);
		if (!registered || registered.provider !== name)
			throw new Error("Provider registration owner mismatch");
		if (hook !== "oauth.modifyModels")
			throw new UnsupportedAPI(`synchronous provider.${hook}`);
		const oauth = registered.config.oauth;
		if (!oauth) throw new UnsupportedAPI(`provider.${hook}`);
		const result = oauth.modifyModels
			? oauth.modifyModels(clone(args.models), clone(args.credentials))
			: clone(args.models);
		if (result && typeof result.then === "function") {
			Promise.resolve(result).catch(() => {});
			throw new UnsupportedAPI(
				"async modifyModels during synchronous registration",
			);
		}
		return clone(result);
	}
	async provider(name, hook, args, registration) {
		const registered =
			registration === undefined
				? this.providers.get(name)
				: this.providerVersions.get(registration);
		if (!registered) throw new Error(`Unknown extension provider: ${name}`);
		if (registered.provider !== name)
			throw new Error("Provider registration owner mismatch");
		const { extension, config } = registered;
		if (hook === "streamSimple") {
			callable(config.streamSimple, "provider streamSimple");
			const stream = config.streamSimple(
				clone(args.model),
				clone(args.context),
				{ ...clone(args.options), signal: this.current?.signal },
			);
			if (!stream?.[Symbol.asyncIterator])
				throw new Error("Provider stream must be async iterable");
			let count = 0,
				bytes = 0;
			for await (const event of stream) {
				bounded(event);
				bytes += Buffer.byteLength(JSON.stringify(event));
				if (++count > 10000 || bytes > 16 * 1024 * 1024)
					throw new Error("Provider stream quota");
				this.emit({
					type: "provider_event",
					operation: this.current?.id,
					provider: name,
					event: clone(event),
				});
				if (this.current?.signal.aborted)
					throw new Error("Extension operation cancelled");
			}
			return typeof stream.result === "function"
				? await stream.result()
				: undefined;
		}
		if (!config.oauth) throw new UnsupportedAPI(`provider.${hook}`);
		if (hook === "oauth.login")
			return config.oauth.login({
				...(args.manualCodeInput === true
					? {
							onManualCodeInput: () =>
								this.request(extension, "oauth.onManualCodeInput", {}),
						}
					: {}),
				onAuth: (info) => this.enqueue(extension, "oauth.onAuth", { info }),
				onPrompt: (prompt) =>
					this.request(extension, "oauth.onPrompt", { prompt }),
				onProgress: (message) =>
					this.enqueue(extension, "oauth.onProgress", { message }),
				signal: this.current?.signal,
			});
		if (hook === "oauth.refreshToken")
			return config.oauth.refreshToken(clone(args.credentials));
		if (hook === "oauth.getApiKey")
			return config.oauth.getApiKey(clone(args.credentials));
		if (hook === "oauth.modifyModels")
			return config.oauth.modifyModels
				? config.oauth.modifyModels(clone(args.models), clone(args.credentials))
				: clone(args.models);
		throw new UnsupportedAPI(`provider.${hook}`);
	}
	aiModule(extension) {
		const stream = (model, context, options = {}) => {
			const events = new AssistantMessageEventStream();
			const { signal, ...data } = options;
			if (signal?.aborted) {
				events.fail(new Error("Extension inference cancelled"));
				return events;
			}
			const abort = () => {
				events.fail(new Error("Extension inference cancelled"));
				if (this.current) this.current.controller.abort();
			};
			if (this.pending.length >= 256) {
				events.fail(new Error("Extension pending action limit"));
				return events;
			}
			signal?.addEventListener("abort", abort, { once: true });
			const promise = this.request(
				extension,
				"inference.stream",
				{ model, context, options: data },
				(event) => {
					bounded(event);
					events.push(event);
				},
				signal,
			);
			// Retain acknowledgement even when extension code abandons the stream.
			const settled = promise
				.then(
					(result) => events.end(result),
					(error) => {
						events.fail(error);
						throw error;
					},
				)
				.finally(() => signal?.removeEventListener("abort", abort));
			settled.catch(() => {});
			this.pending.push(settled);
			return events;
		};

		const complete = (model, context, options = {}) => {
			let promise;
			try {
				const { signal, ...data } = options;
				if (signal?.aborted) throw new Error("Extension inference cancelled");
				if (this.pending.length >= 256)
					throw new Error("Extension pending action limit");
				promise = this.request(
					extension,
					"inference.complete",
					{ model, context, options: data },
					undefined,
					signal,
				);
			} catch (error) {
				promise = Promise.reject(error);
			}
			// Return the same tracked promise. An async wrapper would create a
			// second rejection that becomes unhandled when a caller abandons it.
			promise.catch(() => {});
			if (this.pending.length < 256) this.pending.push(promise);
			return promise;
		};

		return {
			complete,
			completeSimple: complete,
			stream,
			streamSimple: stream,
			AssistantMessageEventStream,
			createAssistantMessageEventStream,

			getModel: (provider, id) =>
				clone(
					this.snapshot.models?.find(
						(m) => m.provider === provider && m.id === id,
					),
				),
			getModels: (provider) =>
				clone(
					this.snapshot.models?.filter((m) => m.provider === provider) ?? [],
				),
		};
	}
	tuiModule() {
		class Text {
			constructor(text = "") {
				this.text = String(text);
			}
			setText(text) {
				this.text = String(text);
			}
			render() {
				return this.text.split("\n");
			}
			invalidate() {}
		}
		class Container {
			constructor() {
				this.children = [];
			}
			addChild(child) {
				if (this.children.length >= 256) throw new Error("Component limit");
				this.children.push(child);
			}
			render(width) {
				return this.children.flatMap((c) => c.render(width));
			}
			invalidate() {}
		}
		class Spacer {
			constructor(lines = 1) {
				this.lines = lines;
			}
			render() {
				return Array.from(
					{ length: Math.min(100, Math.max(0, this.lines)) },
					() => "",
				);
			}
			invalidate() {}
		}
		const theme = {
			fg: (_color, text) => String(text),
			bg: (_color, text) => String(text),
			bold: (text) => String(text),
			italic: (text) => String(text),
			strikethrough: (text) => String(text),
			underline: (text) => String(text),
		};
		return { Text, Container, Spacer, theme };
	}
}
