import { expect, test } from "vitest";
import { createCommandQueue, errorMessage, type View } from "./bridge";
import type { BridgeSnapshot, Command } from "./generated/protocol";
test("back-to-back commands use acknowledged authority without waiting for a React render", async () => {
  let revision = 0n,
    inflight = 0,
    maxInflight = 0;
  const received: string[] = [];
  const snapshot = () =>
    ({
      version: { major: 1, minor: 0, schema: "test" },
      scope: { workspace: "ws", session: "session", instance: "worker", epoch: "1" },
      revision: String(revision),
    }) as BridgeSnapshot;
  const queue = createCommandQueue({
    view: async () => ({ snapshot: snapshot(), error: null, progress: "" }),
    command: async (s, _command, id = crypto.randomUUID()) => {
      inflight++;
      maxInflight = Math.max(maxInflight, inflight);
      expect(s.revision).toBe(String(revision));
      received.push(s.revision);
      await Promise.resolve();
      revision += 2n;
      inflight--;
      return {
        kind: "reply",
        id,
        scope: s.scope,
        revision: String(revision),
        durable: true,
        status: "completed",
        data: null,
      };
    },
  });
  const command: Command = { kind: "model", action: "select", args: { id: "fixture" } };
  await Promise.all(
    Array.from({ length: 20 }, () =>
      queue(command, (_view: View) => {
        /* Deliberately no render or ref update. */
      }),
    ),
  );
  expect(maxInflight).toBe(1);
  expect(received).toEqual(Array.from({ length: 20 }, (_, i) => String(i * 2)));
});
test("a rejected mutation is not replayed and later commands can proceed", async () => {
  let calls = 0;
  const queue = createCommandQueue({
    view: async () => ({
      snapshot: { revision: "1" } as BridgeSnapshot,
      error: null,
      progress: "",
    }),
    command: async () => {
      calls++;
      throw Error("Stale revision");
    },
  });
  const c: Command = { kind: "model", action: "select", args: {} };
  await expect(queue(c, () => {})).rejects.toThrow("Stale revision");
  expect(calls).toBe(1);
  await expect(queue(c, () => {})).rejects.toThrow();
  expect(calls).toBe(2);
  expect(errorMessage(new Error("Error: Error: Stale revision"))).toBe("Stale revision");
});
