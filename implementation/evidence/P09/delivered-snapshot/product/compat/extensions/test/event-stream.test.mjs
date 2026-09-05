import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import { AssistantMessageEventStream } from "../event-stream.mjs";
test("assistant stream terminal/result/consumer ordering matches six actual SDK captures", async () => {
	const oracle = JSON.parse(
		fs.readFileSync(
			new URL(
				"../../../../fixtures/tools/extension-stream-oracle.json",
				import.meta.url,
			),
			"utf8",
		),
	);
	const prior = Date.now;
	Date.now = () => 12;
	try {
		for (const row of oracle.rows) {
			const stream = new AssistantMessageEventStream(),
				events = [];
			let first;
			if (row.waiting) first = stream.next();
			for (const event of row.events) stream.push(event);
			stream.end(row.end);
			if (first) events.push((await first).value);
			for await (const event of stream) events.push(event);
			assert.deepEqual(
				{ events, result: await stream.result() },
				row.output,
				row.id,
			);
		}
	} finally {
		Date.now = prior;
	}
});
test("assistant stream quota and failed transport reject both readers and result without dangling waiters", async () => {
	const stream = new AssistantMessageEventStream();
	const next = stream.next();
	const result = stream.result();
	stream.fail(new Error("transport failed"));
	await assert.rejects(next, /transport failed/);
	await assert.rejects(result, /transport failed/);
	await assert.rejects(stream.next(), /transport failed/);
	const quota = new AssistantMessageEventStream();
	for (let i = 0; i <= 10000; i++)
		quota.push({ type: "text_delta", delta: "x" });
	await assert.rejects(quota.result(), /quota/);
	await assert.rejects(quota.next(), /quota/);
	assert.equal(quota.queue.length, 0);
});
