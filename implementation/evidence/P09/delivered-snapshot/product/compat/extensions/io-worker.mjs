// Only this worker reads/writes the protocol descriptors. Keeping I/O off the
// extension thread lets synchronous SDK registration wait for native authority.
import { Socket } from "node:net";
import { workerData } from "node:worker_threads";

const { port, shared } = workerData;
const state = new Int32Array(shared);
const MAX_FRAME = 1024 * 1024,
	MAX_QUEUE = 4 * MAX_FRAME,
	MAX_COUNT = 4096;
// libuv pipe watchers must remain cancellable: fs streams on pipe descriptors
// use blocking thread-pool reads and can prevent process.exit from joining workers.
const input = new Socket({ fd: 0, readable: true, writable: false });
const output = new Socket({ fd: 1, readable: false, writable: true });
let buffer = Buffer.alloc(0),
	closed = false;
function post(message) {
	port.postMessage(message);
	Atomics.add(state, 0, 1);
	Atomics.notify(state, 0);
}
function close(error) {
	if (closed) return;
	closed = true;
	input.destroy();
	post(
		error
			? { error: String(error?.message ?? error).slice(0, 1024) }
			: { end: true },
	);
	Atomics.store(state, 4, 1);
	Atomics.notify(state, 0);
}
port.on("message", ({ line, bytes }) => {
	if (closed) return;
	output.write(line, (error) => {
		Atomics.sub(state, 1, bytes);
		Atomics.sub(state, 5, 1);
		if (error) close(error);
	});
});
input.on("data", (chunk) => {
	if (closed) return;
	if (buffer.length + chunk.length > MAX_FRAME + 65536) {
		close(new Error("Extension frame quota"));
		return;
	}
	buffer = Buffer.concat([buffer, chunk]);
	while (true) {
		const end = buffer.indexOf(10);
		if (end < 0) break;
		if (end > MAX_FRAME) {
			close(new Error("Extension frame quota"));
			return;
		}
		const line = buffer.subarray(0, end).toString("utf8");
		buffer = buffer.subarray(end + 1);
		if (!end) continue;
		// Count UTF-8 bytes as retained string memory as well as frame count.
		const bytes = Buffer.byteLength(line);
		if (
			Atomics.add(state, 2, bytes) + bytes > MAX_QUEUE ||
			Atomics.add(state, 3, 1) + 1 > MAX_COUNT
		) {
			close(new Error("Extension transport input quota"));
			return;
		}
		post({ line, bytes });
	}
	if (buffer.length > MAX_FRAME) close(new Error("Extension frame quota"));
});
input.on("end", () =>
	close(buffer.length ? new Error("Truncated extension frame") : undefined),
);
input.on("error", close);
output.on("error", close);
process.on("uncaughtException", close);
process.on("unhandledRejection", close);
