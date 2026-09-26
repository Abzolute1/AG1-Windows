import { expect, test, vi } from "vitest";
import { NativeClient, getAgentDir, takeOverStdout } from "../../../clients/sdk";
import type { WireMessage, ClientView, BridgeFrame } from "./generated/protocol";
const scope = { workspace: "w", session: "s", instance: "i", epoch: "1" };
const view = { snapshot: { scope, revision: "1", version: { major: 1, minor: 0, schema: "fixture" } }, progress: "", details: { messages: [] } } as unknown as ClientView;
function fixture() {
  let listener: (frame: WireMessage) => void = () => {};
  const detach = vi.fn(), send = vi.fn(async (_frame: WireMessage) => {});
  const client = new NativeClient({ send, subscribe: fn => { listener = fn; return detach; } });
  const emit = (frame: WireMessage) => listener(frame);
  emit({ type: "client", data: structuredClone(view) });
  const reply = (id: string, changes: Partial<Extract<BridgeFrame, { kind: "reply" }>> = {}) => emit({ type: "bridge", data: { kind: "reply", scope, id, revision: "1", durable: true, status: "accepted", data: {}, ...changes } });
  return { client, emit, send, detach, reply };
}
test("facade correlates concurrent replies by ID and scope", async () => {
  const f = fixture();
  const one = f.client.command({ kind: "cancel", operation: "a" }, "one"), two = f.client.command({ kind: "cancel", operation: "b" }, "two");
  f.reply("one", { scope: { ...scope, epoch: "2" } });
  f.reply("two"); expect((await two).id).toBe("two");
  f.reply("one"); expect((await one).id).toBe("one");
  expect(f.send).toHaveBeenCalledTimes(2);
  f.client.dispose();
});
test("snapshot and listener mutation cannot change command authority", async () => {
  const f = fixture();
  f.client.subscribe(v => { v.snapshot.scope.session = "forged"; });
  f.emit({ type: "client", data: structuredClone(view) });
  f.client.snapshot()!.snapshot.scope.epoch = "999";
  const result = f.client.command({ kind: "cancel", operation: "a" }, "one");
  expect((f.send.mock.calls[0][0] as Extract<WireMessage, { type: "command" }>).data.scope).toEqual(scope);
  f.reply("one"); await result; f.client.dispose();
});
test("detach rejects outstanding waiters without cancelling native work", async () => {
  const f = fixture();
  const result = f.client.command({ kind: "prompt", input: { kind: "inline", text: "work" } }, "one");
  const rejected = expect(result).rejects.toThrow("command_result");
  f.client.dispose(); await rejected;
  await expect(f.client.prompt("again")).rejects.toThrow("detached");
  expect(f.send).toHaveBeenCalledTimes(1); expect(f.detach).toHaveBeenCalledTimes(1);
});
test("rejection remains correlated and unsupported globals fail explicitly", async () => {
  const f = fixture();
  const result = f.client.command({ kind: "cancel", operation: "a" }, "one");
  const rejected = expect(result).rejects.toThrow("stale_context");
  f.reply("one", { status: "rejected", data: { error: "stale_context" } }); await rejected;
  expect(getAgentDir).toThrow("UnsupportedAPI"); expect(takeOverStdout).toThrow("UnsupportedAPI");
  f.client.dispose();
});
