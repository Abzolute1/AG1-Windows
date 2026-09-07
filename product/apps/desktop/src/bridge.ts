import type { Recovery } from "./RecoveryPanel";
import { invoke } from "@tauri-apps/api/core";
import type { BridgeSnapshot, BridgeFrame, Command, CommandEnvelope, ClientDetails } from "./generated/protocol";
export type View = { snapshot: BridgeSnapshot | null; error: string | null; progress: string; details?: ClientDetails };
export type Catalog = { projects: string[]; theme: "light" | "dark" | "system" };
export const api = {
  catalog: () => invoke<Catalog>("catalog"),
  recovery: (path: string) => invoke<Recovery | null>("cutover_status", { path }),
  open: (path: string) => invoke<View>("open_project", { path }),
  view: () => invoke<View>("bridge_view"),
  theme: (theme: string) => invoke<void>("save_preferences", { theme }),
  quit: () => invoke<void>("quit"),
  async command(snapshot: BridgeSnapshot, command: Command, id = crypto.randomUUID()) {
    const envelope: CommandEnvelope = {
      version: snapshot.version,
      id,
      scope: snapshot.scope,
      expected_revision: snapshot.revision,
      command,
    };
    const result = await invoke<Extract<BridgeFrame, { kind: "reply" }>>("bridge_command", {
      envelope,
    });
    if (result.status === "rejected" || result.status === "failed")
      throw new Error(`${(result.data as { detail?: string })?.detail ?? JSON.stringify(result.data)} (command ${id})`);
    return result;
  },
};

/** Obtain authority at dispatch time, independent of React rendering or overlapping clicks. */
export function createCommandQueue(ports: Pick<typeof api, "view" | "command"> = api) {
  let tail: Promise<unknown> = Promise.resolve();
  return (command: Command, onView: (view: View) => void) => {
    const id = crypto.randomUUID();
    const operation = tail.then(async () => {
      const current = await ports.view();
      onView(current);
      if (!current.snapshot) throw new Error("Open a project first");
      try {
        return (await ports.command(current.snapshot, command, id)).data;
      } finally {
        onView(await ports.view());
      }
    });
    tail = operation.catch(() => {});
    return operation;
  };
}
export function errorMessage(error: unknown) {
  return (error instanceof Error ? error.message : String(error)).replace(/^(?:Error:\s*)+/, "");
}
