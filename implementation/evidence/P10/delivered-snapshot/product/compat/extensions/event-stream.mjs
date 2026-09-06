// Bounded optional-host counterpart of the SDK assistant stream. Transport owns
// cancellation/deadlines; reasoning has no implicit idle timer here.
export class AssistantMessageEventStream {
	constructor() {
		this.queue = [];
		this.waiters = [];
		this.done = false;
		this.bytes = 0;
		this.count = 0;
		this.final = new Promise((resolve, reject) => {
			this.resolve = resolve;
			this.reject = reject;
		});
		this.final.catch(() => {});
	}
	push(event) {
		if (this.done) return;
		const text = JSON.stringify(event);
		if (
			text === undefined ||
			++this.count > 10000 ||
			(this.bytes += Buffer.byteLength(text)) > 16 * 1024 * 1024
		) {
			this.fail(new Error("Assistant stream quota exceeded"));
			return;
		}
		const value = JSON.parse(text),
			terminal = value.type === "done" || value.type === "error";
		if (terminal) {
			this.done = true;
			this.resolve(value.type === "done" ? value.message : value.error);
		}
		const waiter = this.waiters.shift();
		if (waiter) waiter.resolve({ value, done: false });
		else this.queue.push(value);
		if (terminal) this.finishWaiters();
	}
	finishWaiters() {
		for (const waiter of this.waiters)
			waiter.resolve({ value: undefined, done: true });
		this.waiters = [];
	}
	end(result) {
		if (this.done) return;
		this.done = true;
		if (result !== undefined) this.resolve(result);
		else
			this.resolve({
				role: "assistant",
				content: [],
				api: "",
				provider: "",
				model: "",
				usage: {
					input: 0,
					output: 0,
					cacheRead: 0,
					cacheWrite: 0,
					totalTokens: 0,
					cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 },
				},
				stopReason: "error",
				errorMessage: "stream ended without completion",
				timestamp: Date.now(),
			});
		this.finishWaiters();
	}
	fail(error) {
		if (this.done) return;
		this.done = true;
		this.failure = error;
		this.queue = [];
		this.reject(error);
		for (const waiter of this.waiters) waiter.reject(error);
		this.waiters = [];
	}
	result() {
		return this.final;
	}
	[Symbol.asyncIterator]() {
		return this;
	}
	next() {
		if (this.failure) return Promise.reject(this.failure);
		if (this.queue.length)
			return Promise.resolve({ value: this.queue.shift(), done: false });
		if (this.done) return Promise.resolve({ value: undefined, done: true });
		if (this.waiters.length >= 256)
			return Promise.reject(new Error("Assistant stream waiter limit"));
		return new Promise((resolve, reject) =>
			this.waiters.push({ resolve, reject }),
		);
	}
}
export function createAssistantMessageEventStream() {
	return new AssistantMessageEventStream();
}
