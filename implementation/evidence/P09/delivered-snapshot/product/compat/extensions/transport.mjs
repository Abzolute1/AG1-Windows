import {
	MessageChannel,
	Worker,
	receiveMessageOnPort,
} from "node:worker_threads";

const MAX_QUEUE = 4 * 1024 * 1024,
	MAX_COUNT = 4096;
export class Transport {
	constructor(receive, fatal) {
		const { port1, port2 } = new MessageChannel();
		this.state = new Int32Array(new SharedArrayBuffer(6 * 4));
		this.port = port1;
		this.receive = receive;
		this.fatal = fatal;
		this.worker = new Worker(new URL("./io-worker.mjs", import.meta.url), {
			workerData: { port: port2, shared: this.state.buffer },
			execArgv: [], // Do not inherit the extension thread's larger heap override.
			transferList: [port2],
			resourceLimits: {
				maxOldGenerationSizeMb: 32,
				maxYoungGenerationSizeMb: 8,
			},
		});
		port1.on("message", (message) => this.accept(message));
		this.worker.on("error", fatal);
		this.worker.on("exit", () => {
			Atomics.store(this.state, 4, 1);
			fatal(new Error("Extension I/O worker stopped"));
		});
	}
	accept(message) {
		if (message.line !== undefined) {
			Atomics.sub(this.state, 2, message.bytes);
			Atomics.sub(this.state, 3, 1);
		}
		this.receive(message);
	}
	send(line) {
		if (Atomics.load(this.state, 4))
			throw new Error("Extension transport closed");
		const bytes = Buffer.byteLength(line);
		const queued = Atomics.add(this.state, 1, bytes) + bytes;
		const count = Atomics.add(this.state, 5, 1) + 1;
		if (queued > MAX_QUEUE || count > MAX_COUNT) {
			Atomics.sub(this.state, 1, bytes);
			Atomics.sub(this.state, 5, 1);
			throw new Error("Extension transport output quota");
		}
		this.port.postMessage({ line, bytes });
	}
	// Pump only native protocol messages while the extension's stack is blocked.
	// Native operation deadlines still supervise and can terminate the whole host.
	pumpUntil(done) {
		const deadline = performance.now() + 60000;
		while (!done()) {
			const sequence = Atomics.load(this.state, 0);
			const next = receiveMessageOnPort(this.port);
			if (next) {
				this.accept(next.message);
				continue;
			}
			if (Atomics.load(this.state, 4))
				throw new Error("Extension transport closed");
			if (performance.now() >= deadline)
				throw new Error("Synchronous extension callback deadline");
			Atomics.wait(this.state, 0, sequence, 100);
		}
	}
}
