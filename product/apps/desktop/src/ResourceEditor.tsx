import { useEffect, useRef, useState } from "react";
import type { Command, JsonValue } from "./generated/protocol";
import { Button } from "./components/ui/button";
type File = { path: string; text: string; revision: string };
export function ResourceEditor({ path, run, close }: { path: string; run: (command: Command) => Promise<JsonValue>; close: () => void }) {
  const [file, setFile] = useState<File | null>(null), [text, setText] = useState(""), [busy, setBusy] = useState(false), [error, setError] = useState("");
  const current = useRef(text); current.current = text;
  useEffect(() => { let closed = false; void run({ kind: "files", action: "read", args: { path } }).then(value => { if (!closed) { const file = value as unknown as File; setFile(file); setText(file.text); } }).catch(e => { if (!closed) setError(String(e)); }); return () => { closed = true; }; }, [path, run]);
  async function save() {
    if (!file || busy) return;
    setBusy(true); setError(""); const sent = text;
    try { const saved = await run({ kind: "files", action: "save", args: { path, revision: file.revision, text: sent } }) as unknown as File; setFile(saved); if (current.current === sent) setText(saved.text); } catch (e) { setError(String(e)); } finally { setBusy(false); }
  }
  const dirty = !!file && file.text !== text;
  return <section aria-label="Resource editor"><h4>{path}</h4>{error && <p role="alert">{error}</p>}{file ? <><textarea aria-label="Resource content" value={text} onChange={e => setText(e.target.value)} /><Button disabled={busy || !dirty} onClick={() => void save()}>Save resource</Button>{dirty && <span>Unsaved resource changes</span>}</> : <p>Loading resource…</p>}<Button disabled={busy} onClick={close}>{dirty ? "Discard resource changes" : "Close resource"}</Button><p>Changes apply when resources are next loaded. An active run keeps its admitted resource snapshot.</p></section>;
}
