import { expect, test, vi } from "vitest";
import { navigateSession } from "./sessionNavigation";
import type { JsonValue } from "./generated/protocol";
test("navigation restores full paged user text only after the matching operation commits", async () => {
  let resolve!: (value: JsonValue) => void;
  const restored = vi.fn();
  const run = vi.fn(async (command: any): Promise<JsonValue> => {
    if (command.action === "user_text") return command.args.offset === "0" ? { base64: btoa("first "), next: "6" } : { base64: btoa("second"), next: null };
    if (command.action === "fork") return { operation: "fork-1" };
    return new Promise(done => { resolve = done; });
  });
  const pending = navigateSession(run, "fork", { id: "user", role: "user" }, false, restored);
  await vi.waitFor(() => expect(resolve).toBeTypeOf("function"));
  expect(restored).not.toHaveBeenCalled();
  resolve({ last_terminal: { operation: "fork-1", status: "completed" } }); await pending;
  expect(restored).toHaveBeenCalledExactlyOnceWith("first second");
});
test("failed navigation keeps the current input and exposes the native terminal result", async () => {
  const restored = vi.fn();
  const run = async (command: any): Promise<JsonValue> => command.action === "user_text" ? { base64: btoa("original"), next: null } : command.action === "tree" ? { operation: "tree-1" } : { last_terminal: { operation: "tree-1", status: "failed" } };
  await expect(navigateSession(run, "tree", { id: "user", role: "user" }, true, restored)).rejects.toThrow("Navigation failed");
  expect(restored).not.toHaveBeenCalled();
});
