import type { Recovery } from "./RecoveryPanel";
import { invoke } from "@tauri-apps/api/core";
import type { BridgeSnapshot, BridgeFrame, Command, CommandEnvelope, ClientDetails } from "./generated/protocol";
export type View = { snapshot: BridgeSnapshot | null; error: string | null; progress: string; details?: ClientDetails };
export type ConversationStatus = { session: string; workspace: string; status: string; pending: number; error: string | null };
export type Chat = { session: string; workspace: string; title: string; updated?: string; archived?: boolean; deleted?: boolean; renamed?: boolean; general?: boolean; empty?: boolean };
export type ReasoningControls = { kind: "toggle" | "levels" | "unknown" | "unsupported"; modes: { id: string; label: string; level: string }[]; default?: string; source: string };
export type Model = { input?: string[]; id: string; provider: string; contextWindow?: number; reasoning?: boolean; reasoningControls?: ReasoningControls; conflict?: unknown };
export type NativeModelSetup = { modelId: string; serverPath: string; modelPath: string; profilePath: string; projectorPath?: string; port: number; vramMiB?: number; systemPrompt?: string };
export type Catalog = { projects: string[]; theme: import("./themes").Theme; chats?: Chat[]; themes?: import("./themes").ThemePlugin[] };
export const api = {
  chooseChatFolder: () => invoke<string | null>("choose_chat_folder"),
  selectConversation: (session?: string) => invoke<View>("select_conversation", { session: session ?? null }),
  conversationStatuses: () => invoke<ConversationStatus[]>("conversation_statuses"),
  reasoning: (provider: string, id: string) => invoke<ReasoningControls>("model_reasoning", { provider, id }),
  exportTheme: (plugin: unknown) => invoke<string>("export_theme", { plugin }),
  importTheme: (plugin: unknown) => invoke<void>("import_theme", { plugin }),
  removeTheme: (id: string) => invoke<void>("remove_theme", { id }),
  github: (action: string, args: Record<string, unknown> = {}) => invoke<any>("github_command", { action, args }),
  preferences: () => invoke<{ approval_mode: string }>("desktop_preferences"),
  models: () => invoke<Model[]>("desktop_models"),
  configureNativeModel: (setup: NativeModelSetup) => invoke<Model[]>("configure_native_model", { setup }),
  chooseNativeRuntimeFile: (kind: "server" | "model" | "projector" | "profile") => invoke<string | null>("choose_native_runtime_file", { kind }),
  general: () => invoke<View>("open_general"),
  indexChat: (chat: Chat) => invoke<void>("save_chat", { chat: { ...chat, discovered: true } }),
  saveChat: (chat: Chat) => invoke<void>("save_chat", { chat }),
  catalog: () => invoke<Catalog>("catalog"),
  recovery: (path: string) => invoke<Recovery | null>("cutover_status", { path }),
  open: (path: string) => invoke<View>("open_project", { path }),
  live: (after: number) => invoke<any>("bridge_live", { after }),
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
    if (result.status === "rejected" || result.status === "failed") {
      const data = result.data as { error?: unknown; detail?: string } | null;
      // Translate only the native editor-save conflict. Other commands and
      // errors retain their diagnostics; never retry or relax the revision.
      const detail = command.kind === "files" && command.action === "save" && data?.error === "Conflict"
        ? "Save conflict: the file changed on disk. Your unsaved text is still in the editor. Copy it somewhere safe before choosing Discard. Reopen the file from the file list to load the current version, then reapply your changes."
        : data?.detail ?? JSON.stringify(result.data);
      throw new Error(`${detail} (command ${id})`);
    }
    return result;
  },
};

/** Obtain authority at dispatch time, independent of React rendering or overlapping clicks. */
export function createCommandQueue(ports: Pick<typeof api, "view" | "command"> = api) {
  let tail: Promise<unknown> = Promise.resolve();
  return (command: Command, onView: (view: View) => void, expectedScope?: BridgeSnapshot["scope"]) => {
    const id = crypto.randomUUID();
    const operation = tail.then(async () => {
      const current = await ports.view();
      onView(current);
      if (!current.snapshot) throw new Error("Open a project first");
      if (expectedScope && JSON.stringify(current.snapshot.scope) !== JSON.stringify(expectedScope)) throw new Error("The conversation changed before this request could start");
      try {
        let result = (await ports.command(current.snapshot, command, id)).data;
        // Navigation is admitted before the new session is published. Hold the
        // queue until that exact durable receipt settles, including across the
        // scope change; otherwise the next prompt can enter yesterday's session.
        if (command.kind === "session" && ["new", "resume", "fork"].includes(command.action)) {
          const deadline = Date.now() + 60000;
          while ((result as any)?.kind === "accepted") {
            if (Date.now() >= deadline) throw new Error("Chat navigation is still pending. Your message has not been sent; wait for navigation to finish before retrying.");
            await new Promise(resolve => setTimeout(resolve, 40));
            result = (await ports.command(current.snapshot, { kind: "command_result", command: id })).data;
          }
          if ((result as any)?.kind === "settled" && (result as any).operation.status !== "completed") throw new Error(`Chat navigation ${(result as any).operation.status}. Your message has not been sent.`);
        }
        return result;
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
