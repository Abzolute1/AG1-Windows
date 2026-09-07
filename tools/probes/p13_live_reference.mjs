// Called only by the native canary owner while its generation permit and
// authenticated runtime handle remain held. This helper never starts a model.
import fs from "node:fs";
import assert from "node:assert/strict";
import path from "node:path";
import { pathToFileURL } from "node:url";
const [source, inputPath, outputPath] = process.argv.slice(2);
const input = JSON.parse(fs.readFileSync(inputPath, "utf8"));
const { request, expected_payload: expected, origin } = input;
assert.equal(new URL(request.model.baseUrl).origin, origin);
assert(["localhost", "127.0.0.1", "[::1]"].includes(new URL(origin).hostname));
assert.equal(request.model.api, "openai-completions");
const { streamOpenAICompletions } = await import(pathToFileURL(path.join(source, "packages/ai/dist/providers/openai-completions.js")));
const started = performance.now();
let first = null, payload = null, requestedCancel = false;
const controller = new AbortController();
const timeout = setTimeout(() => controller.abort(), 900000);
try {
  const stream = streamOpenAICompletions(request.model, request.context, {
    ...request.options, apiKey: "p13-local-no-credential", signal: controller.signal,
    onPayload(value) { assert.deepEqual(value, expected); payload = structuredClone(value); },
  });
  for await (const event of stream) {
    if (["text_delta", "thinking_delta", "toolcall_delta"].includes(event.type) && event.delta?.length) {
      if (first === null) first = Math.floor(performance.now() - started);
      if (request.cancel_after_first_content) { requestedCancel = true; controller.abort(); }
    }
  }
  const message = await stream.result();
  assert(payload, "Pinned TS payload comparison did not complete");
  fs.writeFileSync(outputPath, JSON.stringify({ message, payload,
    first_content_ms: first, elapsed_ms: Math.floor(performance.now() - started),
    cancelled: requestedCancel && message.stopReason === "aborted", settled: false,
  }), { flag: "wx", mode: 0o600 });
} finally { clearTimeout(timeout); }
