import { useEffect, useState } from "react";
import { navigateSession } from "./sessionNavigation";
import { Button } from "./components/ui/button";
import type { Command, JsonValue } from "./generated/protocol";
type Entry = { id: string; parent: string | null; kind: string; role?: string; preview?: string };
type History = { entries: Entry[]; total: number; next: number | null };
export function SessionHistory({ run, onEditor }: { run: (command: Command) => Promise<JsonValue>; onEditor: (text: string) => void }) {
  const [history, setHistory] = useState<History | null>(null), [error, setError] = useState(""), [summarize, setSummarize] = useState(false), [filename, setFilename] = useState("conversation.html"), [exported, setExported] = useState("");
  async function load(offset = 0) { try { setHistory(await run({ kind: "session", action: "history", args: { offset } }) as unknown as History); } catch (e) { setError(String(e)); } }
  useEffect(() => { void load(); }, [run]);
  async function execute(action: string, args: JsonValue) { try { setError(""); return await run({ kind: "session", action, args }); } catch (e) { setError(String(e)); return null; } }
  return <section aria-label="Session history"><h3>Session history and branches</h3>{error && <p role="alert">{error}</p>}<Button onClick={() => void load()}>Refresh history</Button><label><input type="checkbox" checked={summarize} onChange={e => setSummarize(e.target.checked)} />Summarize the abandoned branch when navigating</label>{history?.entries.map(entry => <article key={entry.id}><strong>{entry.role ?? entry.kind}</strong><p>{entry.preview}</p><small>{entry.id} · parent {entry.parent ?? "root"}</small><div><Button onClick={() => void navigateSession(run, "tree", entry, summarize, onEditor).then(() => load()).catch(e => setError(String(e)))}>Navigate here</Button>{entry.role === "user" && <Button onClick={() => void navigateSession(run, "fork", entry, false, onEditor).then(() => load()).catch(e => setError(String(e)))}>Fork before this message</Button>}</div></article>)}{history?.next !== null && history?.next !== undefined && <Button onClick={() => void load(history.next!)}>Next history page</Button>}
    <h3>HTML export</h3><form onSubmit={e => { e.preventDefault(); void execute("export", { path: filename }).then(result => { if (result) setExported(`Saved ${(result as { path: string }).path}`); }); }}><label>New filename in this project<input required value={filename} onChange={e => setFilename(e.target.value)} /></label><Button type="submit">Export conversation</Button></form>{exported && <p role="status">{exported}</p>}<p>Export includes the complete active-branch text and license notice. Attachment bytes stay in the native journal. Existing files are preserved.</p>
  </section>;
}
