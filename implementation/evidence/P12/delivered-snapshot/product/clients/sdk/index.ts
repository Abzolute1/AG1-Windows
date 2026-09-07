/** A protocol client. The embedding host owns transport and native session lifetime. */
import type { BridgeFrame, ClientView, Command, CommandEnvelope, Scope, WireMessage } from "../../apps/desktop/src/generated/protocol";
export type Transport = { send(frame: WireMessage): Promise<void>; subscribe(listener: (frame: WireMessage) => void): () => void };
type Reply = Extract<BridgeFrame, { kind: "reply" }>;
type Pending = { scope: Scope; resolve: (reply: Reply) => void; reject: (error: Error) => void };
export class UnsupportedAPI extends Error {
  constructor(name: string) { super(`UnsupportedAPI: ${name}. Use the explicit native protocol; see COMPATIBILITY.md.`); this.name = "UnsupportedAPI"; }
}
function sameScope(a: Scope, b: Scope) {
  return a.workspace === b.workspace && a.session === b.session && a.instance === b.instance && a.epoch === b.epoch;
}
export class NativeClient {
  private view: ClientView | null = null;
  private pending = new Map<string, Pending>();
  private listeners = new Set<(view: ClientView) => void>();
  private detach: () => void;
  private disposed = false;
  constructor(private transport: Transport) {
    this.detach = transport.subscribe(frame => {
      if (this.disposed) return;
      if (frame.type === "client") {
        this.view = structuredClone(frame.data);
        for (const listener of this.listeners) listener(structuredClone(this.view));
      }
      if (frame.type === "bridge" && frame.data.kind === "reply") {
        const pending = this.pending.get(frame.data.id);
        // IDs belong to a scope. A reply from another host/session is not an acknowledgement.
        if (!pending || !sameScope(pending.scope, frame.data.scope)) return;
        this.pending.delete(frame.data.id);
        if (["failed", "rejected"].includes(frame.data.status)) pending.reject(new Error(JSON.stringify(frame.data.data)));
        else pending.resolve(structuredClone(frame.data));
      }
      if (frame.type === "error") {
        // Framing errors have no command correlation. They cannot settle a command.
        // The caller can inspect durable command_result after reconnecting.
        this.failPending(new Error(`Transport framing error; command outcome unknown: ${JSON.stringify(frame.data)}`));
      }
    });
  }
  private failPending(error: Error) { for (const pending of this.pending.values()) pending.reject(error); this.pending.clear(); }
  snapshot(): ClientView | null { return this.view === null ? null : structuredClone(this.view); }
  subscribe(listener: (view: ClientView) => void): () => void {
    if (this.disposed) throw new Error("Client detached");
    this.listeners.add(listener); if (this.view) listener(structuredClone(this.view));
    return () => this.listeners.delete(listener);
  }
  async command(command: Command, id: string = crypto.randomUUID()): Promise<Reply> {
    if (this.disposed) throw new Error("Client detached");
    const view = this.view; if (!view) throw new Error("Await the host's initial snapshot");
    if (this.pending.size >= 256) throw new Error("Client command queue full");
    if (this.pending.has(id)) throw new Error("Command ID already in flight");
    const envelope: CommandEnvelope = structuredClone({ version: view.snapshot.version, scope: view.snapshot.scope, expected_revision: view.snapshot.revision, id, command });
    return new Promise<Reply>((resolve, reject) => {
      this.pending.set(id, { scope: envelope.scope, resolve, reject });
      try {
        this.transport.send({ type: "command", data: envelope }).catch(error => { this.pending.delete(id); reject(error); });
      } catch (error) { this.pending.delete(id); reject(error); }
    });
  }
  prompt(text: string) { return this.command({ kind: "prompt", input: { kind: "inline", text } }); }
  cancel(operation: string) { return this.command({ kind: "cancel", operation }); }
  steer(text: string) { return this.command({ kind: "session", action: "steer", args: { input: { kind: "inline", text } } }); }
  /** Detach presentation only. Explicit cancel and host shutdown are separate. */
  dispose() {
    if (this.disposed) return;
    this.disposed = true; this.detach(); this.listeners.clear();
    this.failPending(new Error("Client detached; query command_result by ID before retrying"));
  }
}
/** Named convenience only; this is not the legacy TypeScript AgentSession API. */
export function createAgentSession(transport: Transport) { return new NativeClient(transport); }
export function getAgentDir(): never { throw new UnsupportedAPI("getAgentDir ambient global"); }
export function takeOverStdout(): never { throw new UnsupportedAPI("takeOverStdout terminal ownership"); }
