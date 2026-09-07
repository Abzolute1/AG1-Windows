import { useMemo, useRef, useState } from "react";
import { Virtuoso } from "react-virtuoso";
import { Blocks } from "./NativeDetails";
import { Button } from "./components/ui/button";
import type { BridgeSnapshot, ClientDetails, ClientMessage, Command, JsonValue } from "./generated/protocol";
type Page = { messages: ClientMessage[]; next_before: number | null; start: number; total: number };
export function NativeConversation({ details, snapshot, run }: { details: ClientDetails; snapshot: BridgeSnapshot; run: (command: Command) => Promise<JsonValue> }) {
  const [history, setHistory] = useState<ClientMessage[]>([]), [before, setBefore] = useState<number | null | undefined>(undefined), [busy, setBusy] = useState(false), [error, setError] = useState("");
  const [entry, setEntry] = useState<{ id: string; text: string; next: string | null } | null>(null);
  const decoder = useRef(new TextDecoder());
  const messages = useMemo(() => {
    const combined = new Map(history.map(m => [m.id, m]));
    for (const message of details.messages) combined.set(message.id, message);
    return [...combined.values()];
  }, [history, details.messages]);
  const remaining = before === undefined ? Math.max(0, snapshot.message_count - messages.length) : before ?? 0;
  async function earlier() {
    if (busy || !remaining) return;
    setBusy(true); setError("");
    try {
      const page = await run({ kind: "session", action: "messages", args: { before: remaining } }) as unknown as Page;
      setHistory(prior => [...page.messages, ...prior]); setBefore(page.next_before);
    } catch (e) { setError(String(e)); } finally { setBusy(false); }
  }
  async function evidence(id: string, offset = "0") {
    setError("");
    try {
      if (offset === "0") decoder.current = new TextDecoder();
      const page = await run({ kind: "session", action: "entry", args: { id, offset } }) as { base64: string; next: string | null };
      const text = decoder.current.decode(Uint8Array.from(atob(page.base64), c => c.charCodeAt(0)), { stream: page.next !== null });
      setEntry({ id, text, next: page.next });
    } catch (e) { setError(String(e)); }
  }
  return <section className="native-conversation" aria-label="Conversation">
    <div className="history-controls"><span>{messages.length} of {snapshot.message_count} messages loaded</span>{remaining > 0 && <Button disabled={busy} onClick={() => void earlier()}>{busy ? "Loading history…" : "Load earlier messages"}</Button>}</div>
    {error && <p role="alert">{error}</p>}
    {messages.length ? <Virtuoso className="messages" data={messages} firstItemIndex={Math.max(0, snapshot.message_count - messages.length)} computeItemKey={(_, m) => m.id} initialTopMostItemIndex={{ index: "LAST", align: "end" }} followOutput="auto" itemContent={(_, m) => <article className={`message ${m.role}`}><div className="message-avatar">{m.role === "user" ? "You" : m.role === "assistant" ? "a1" : "Tool"}</div><div className="message-body"><strong>{m.role === "user" ? "You" : m.role === "assistant" ? "AG1" : m.role}</strong><Blocks message={m} /><button className="evidence-button" onClick={() => void evidence(m.id)}>View evidence</button></div></article>} /> : <div className="chat-welcome"><span className="welcome-mark">a1</span><h2>Ready when you are.</h2><p>Ask a question, explore the code, or describe a change.</p></div>}
    {entry && <aside className="entry-view" aria-label="Journal evidence"><header><strong>Journal evidence</strong><Button onClick={() => setEntry(null)}>Close evidence</Button></header><pre>{entry.text}</pre>{entry.next && <Button onClick={() => void evidence(entry.id, entry.next!)}>Next evidence page</Button>}</aside>}
  </section>;
}
