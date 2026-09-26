import type { Command, JsonValue } from "./generated/protocol";
type Run = (command: Command) => Promise<JsonValue>;
/** Restore the selected user's text only after native navigation commits. */
export async function navigateSession(run: Run, action: "tree" | "fork", entry: { id: string; role?: string }, summarize: boolean, onEditor: (text: string) => void) {
  let text = "", offset: string | null = "0"; const decoder = new TextDecoder();
  if (entry.role === "user") do {
    const page = await run({ kind: "session", action: "user_text", args: { id: entry.id, offset } }) as { base64: string; next: string | null };
    text += decoder.decode(Uint8Array.from(atob(page.base64), c => c.charCodeAt(0)), { stream: page.next !== null }); offset = page.next;
    if (text.length > 4 * 1024 * 1024) throw new Error("Input exceeds editor limit; read the original journal entry.");
  } while (offset !== null);
  const result = await run({ kind: "session", action, args: action === "tree" ? { target: entry.id, summarize } : { entry: entry.id } }) as { operation: string | { operation: string } };
  const operation = typeof result.operation === "string" ? result.operation : result.operation.operation;
  const until = Date.now() + 300000;
  while (Date.now() < until) {
    const state = await run({ kind: "runtime", action: "status", args: {} }) as { last_terminal?: { operation: string; status: string } };
    if (state.last_terminal?.operation === operation) {
      if (state.last_terminal.status !== "completed") throw new Error(`Navigation ${state.last_terminal.status}`);
      if (entry.role === "user") onEditor(text); return;
    }
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  throw new Error("Navigation is still pending; inspect Activity before another action.");
}
