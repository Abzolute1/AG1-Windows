// Versioned private stdin/stdout protocol; launched only by the native owner.
import { Console } from "node:console";
import { ExtensionRuntime, UnsupportedAPI, bounded } from "./runtime.mjs";
import { Transport } from "./transport.mjs";
import { SnapshotTransfer } from "./snapshot-transfer.mjs";
globalThis.console = new Console({
	stdout: process.stderr,
	stderr: process.stderr,
});
const MAX_FRAME = 1024 * 1024,
	MAX_PENDING = 256;
let runtime,
	active,
	serial = 0,
	syncDepth = 0,
	closed = false;
const pending = new Map();
const invocations = new Map();
const snapshots = new SnapshotTransfer();
let invoked = new Set();
function fatal(error) {
	if (closed) return;
	closed = true;
	process.stderr.write(
		`Extension host transport: ${String(error?.message ?? error).slice(0, 1024)}\n`,
	);
	process.exitCode = 70;
	process.stdin.destroy();
	for (const callback of pending.values())
		callback.reject(new Error("Extension host closed"));
	pending.clear();
	setImmediate(() => process.exit(70));
}
function sendNow(frame) {
	if (closed) throw new Error("Extension transport closed");
	bounded(frame, MAX_FRAME);
	transport.send(JSON.stringify(frame) + "\n");
}
async function send(frame) {
	sendNow(frame);
}
function packet(value) {
	return value === undefined ? { defined: false } : { defined: true, value };
}
function unpack(value) {
	if (!value || typeof value.defined !== "boolean")
		throw new Error("Invalid extension result packet");
	return value.defined ? value.value : undefined;
}
async function bridge(call, onEvent, signal) {
	if (pending.size >= MAX_PENDING)
		throw new Error("Extension native request limit");
	const id = `native-${++serial}`;
	let callback;
	const promise = new Promise((resolve, reject) => {
		callback = {
			resolve,
			reject,
			onEvent,
			operation: call.operation,
			invocation: call.invocation,
		};
	});
	pending.set(id, callback);
	const abort = () => {
		send({ v: 1, kind: "call_cancel", id }).catch(fatal);
	};
	signal?.addEventListener("abort", abort, { once: true });
	try {
		await send({ v: 1, kind: "call", id, ...call });
		if (signal?.aborted) abort();
		return await promise;
	} finally {
		pending.delete(id);
		signal?.removeEventListener("abort", abort);
	}
}
function bridgeSync(call, signal) {
	if (pending.size >= MAX_PENDING || syncDepth >= 4)
		throw new Error("Extension native request limit");
	const id = `native-${++serial}`;
	let settled = false,
		result,
		failure;
	pending.set(id, {
		operation: call.operation,
		invocation: call.invocation,
		resolve: (value) => {
			result = value;
			settled = true;
		},
		reject: (error) => {
			failure = error;
			settled = true;
		},
	});
	const abort = () => sendNow({ v: 1, kind: "call_cancel", id });
	signal?.addEventListener("abort", abort, { once: true });
	syncDepth++;
	try {
		sendNow({ v: 1, kind: "call", id, ...call });
		if (signal?.aborted) abort();
		transport.pumpUntil(() => settled || closed);
		if (!settled) throw new Error("Extension transport closed");
		if (failure) throw failure;
		return result;
	} catch (error) {
		// A failed wait leaves acknowledgement uncertain: never reuse this host.
		if (!settled) fatal(error);
		throw error;
	} finally {
		syncDepth--;
		pending.delete(id);
		signal?.removeEventListener("abort", abort);
	}
}
function dispatch(frame) {
	if (frame.v !== 1 || typeof frame.id !== "string" || frame.id.length > 256)
		throw new Error("Invalid extension protocol envelope");

	if (frame.kind === "stream") {
		const callback = pending.get(frame.id);
		if (!callback?.onEvent) throw new Error("Unmatched native stream event");
		bounded(frame.event);
		callback.onEvent(frame.event);
		return;
	}
	if (frame.kind === "response") {
		const callback = pending.get(frame.id);
		if (!callback) throw new Error("Unmatched or duplicate native response");
		pending.delete(frame.id);
		if (frame.error)
			callback.reject(
				new Error(String(frame.error.message ?? "Native request rejected")),
			);
		else callback.resolve(unpack(frame.result));
		return;
	}
	if (frame.kind === "cancel") {
		runtime?.cancel(frame.id);
		return;
	}
	if (frame.kind === "invoke") {
		const parent = pending.get(frame.parent);
		const depth = parent?.invocation
			? (invocations.get(parent.invocation)?.depth ?? 4) + 1
			: 1;
		if (
			!active ||
			parent?.operation !== active ||
			frame.method !== "provider" ||
			depth > 4 ||
			invoked.size >= 256 ||
			invoked.has(frame.id)
		)
			throw new Error("Invalid nested provider invocation");
		bounded(frame.args);
		if (typeof frame.args?.registration !== "string")
			throw new Error("Nested provider registration required");
		invoked.add(frame.id);
		invocations.set(frame.id, { depth });
		const args = frame.args;
		const reply = (result, error, failed = false) => {
			invocations.delete(frame.id);
			sendNow({
				v: 1,
				kind: "invoke_response",
				id: frame.id,
				...(failed
					? {
							error: {
								code: error?.code ?? "extension_error",
								message: String(error?.message ?? error).slice(0, 8192),
							},
						}
					: { result: packet(result) }),
			});
		};
		// Validation and synchronous callbacks must throw on this stack; rejected
		// Promise handlers cannot run while a synchronous native wait is pumping.
		if (syncDepth) {
			try {
				const result = runtime.invocations.run(frame.id, () =>
					runtime.providerSync(
						args.name,
						args.hook,
						args.arguments,
						args.registration,
					),
				);
				reply(result);
			} catch (error) {
				reply(undefined, error, true);
			}
			return;
		}
		return (async () => {
			try {
				// Nested hooks keep the parent signal and must not flush parent work.
				const result = await runtime.invocations.run(frame.id, () =>
					runtime.provider(
						args.name,
						args.hook,
						args.arguments,
						args.registration,
					),
				);
				reply(result);
			} catch (error) {
				reply(undefined, error, true);
			}
		})();
	}
	if (frame.kind !== "request" || typeof frame.method !== "string")
		throw new Error("Invalid extension request");
	return dispatchRequest(frame);
}
async function dispatchRequest(frame) {
	if (active) {
		await send({
			v: 1,
			kind: "response",
			id: frame.id,
			error: {
				code: "busy",
				message: "Extension host already has an active operation",
			},
		});
		return;
	}
	active = frame.id;
	invoked = new Set();
	try {
		bounded(frame.args);
		const args = frame.args ?? {};
		if (args.snapshotRef !== undefined) {
			if (args.snapshot !== undefined || !["bind", "event", "tool.prepare", "tool.execute", "command", "shortcut", "provider"].includes(frame.method))
				throw new Error("Invalid native snapshot reference");
			args.snapshot = snapshots.take(args.snapshotRef);
		}
		let result;
		if (frame.method === "initialize") {
			if (runtime) throw new Error("Extension host already initialized");
			runtime = new ExtensionRuntime(bridge, {
				bridgeSync,
				cwd: args.cwd,
				emit: (event) => {
					send({
						v: 1,
						kind: "event",
						id: active,
						invocation: runtime?.invocations.getStore(),
						event,
					}).catch(fatal);
				},
			});
			result = { protocol: 1, processId: process.pid };
		} else {
			if (!runtime) throw new Error("Extension host not initialized");
			switch (frame.method) {
				case "snapshot.begin":
					result = snapshots.begin(args);
					break;
				case "snapshot.append":
					result = snapshots.append(args);
					break;
				case "load":
					result = await runtime.load(args.entries);
					break;
				case "bind":
					result = await runtime.bind(args.snapshot ?? {}, frame.id);
					break;
				case "catalog":
					result = runtime.catalog();
					break;
				case "models.update":
					runtime.updateProviderModels(args);
					runtime.retainProviderVersions(args.retainRegistrations);
					result = { generation: args.generation };
					break;
				case "flags.set":
					runtime.setFlagValue(args.name, args.value);
					result = runtime.getFlagValues();
					break;
				case "flags.values":
					result = runtime.getFlagValues();
					break;
				case "shortcuts.resolve":
					result = runtime.shortcuts(args.keybindings);
					break;
				case "shortcut":
					result = await runtime.invoke(
						frame.id,
						() => runtime.shortcut(args.key, args.keybindings),
						args.snapshot,
					);
					break;

				case "event":
					result = await runtime.invoke(
						frame.id,
						() => runtime.event(args.event),
						args.snapshot,
					);
					break;
				case "tool.prepare":
					result = await runtime.invoke(
						frame.id,
						() => runtime.prepareTool(args.name, args.arguments),
						args.snapshot,
					);
					break;
				case "tool.execute":
					result = await runtime.invoke(
						frame.id,
						() => runtime.executeTool(args.name, args.callId, args.arguments),
						args.snapshot,
					);
					break;
				case "command":
					result = await runtime.invoke(
						frame.id,
						() => runtime.command(args.name, args.arguments),
						args.snapshot,
					);
					break;
				case "provider":
					result = await runtime.invoke(
						frame.id,
						() =>
							runtime.provider(
								args.name,
								args.hook,
								args.arguments,
								args.registration,
							),
						args.snapshot,
					);
					break;
				default:
					throw new UnsupportedAPI(`host.${frame.method}`);
			}
		}
		if (invocations.size) {
			fatal(new Error("Parent operation ended during a provider invocation"));
			return;
		}
		await send({
			v: 1,
			kind: "response",
			id: frame.id,
			result: packet(result),
		});
	} catch (error) {
		if (invocations.size) {
			fatal(new Error("Parent operation failed during a provider invocation"));
			return;
		}
		await send({
			v: 1,
			kind: "response",
			id: frame.id,
			error: {
				code: error?.code ?? "extension_error",
				message: String(error?.message ?? error).slice(0, 8192),
			},
		});
	} finally {
		active = undefined;
	}
}
const transport = new Transport((message) => {
	if (closed) return;
	if (message.error) {
		fatal(new Error(message.error));
		return;
	}
	if (message.end) {
		closed = true;
		for (const callback of pending.values())
			callback.reject(new Error("Native owner disconnected"));
		pending.clear();
		setImmediate(() => process.exit(0));
		return;
	}
	try {
		const frame = JSON.parse(message.line);
		bounded(frame);
		dispatch(frame)?.catch(fatal);
	} catch (error) {
		fatal(error);
	}
}, fatal);
process.on("uncaughtException", fatal);
process.on("unhandledRejection", fatal);
