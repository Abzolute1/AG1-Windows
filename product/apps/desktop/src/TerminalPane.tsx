import { useEffect, useRef, useState } from "react";
import { Button } from "./components/ui/button";
import type { Command, JsonValue } from "./generated/protocol";
type Status = { open: boolean; text: string; sequence: number };
export default function TerminalPane({ run }: { run: (command: Command) => Promise<JsonValue> }) {
  const [status, setStatus] = useState<Status | null>(null), [line, setLine] = useState(""), [error, setError] = useState("");
  const element = useRef<HTMLPreElement>(null);
  async function execute(action: string, args: JsonValue = {}) { try { setError(""); return await run({ kind: "runtime", action: `terminal_${action}`, args }); } catch (e) { setError(String(e)); return null; } }
  useEffect(() => {
    let closed = false, busy = false;
    const poll = async () => { if (busy) return; busy = true; try { const next = await run({ kind: "runtime", action: "terminal_status", args: {} }); if (!closed) setStatus(next as Status); } catch (e) { if (!closed) setError(String(e)); } finally { busy = false; } };
    void poll(); const timer = setInterval(poll, 100); return () => { closed = true; clearInterval(timer); };
  }, [run]);
  useEffect(() => { if (element.current) element.current.scrollTop = element.current.scrollHeight; }, [status?.sequence]);
  useEffect(() => {
    if (!status?.open || !element.current) return;
    const observer = new ResizeObserver(([entry]) => { void execute("resize", { rows: Math.max(1, Math.min(1000, Math.floor(entry.contentRect.height / 18))), columns: Math.max(1, Math.min(1000, Math.floor(entry.contentRect.width / 8))) }); });
    observer.observe(element.current); return () => observer.disconnect();
  }, [status?.open, run]);
  return <section aria-label="Workspace terminal"><h3>Workspace terminal</h3>{error && <p role="alert">{error}</p>}{!status?.open ? <Button onClick={() => void execute("start")}>Start terminal</Button> : <><Button onClick={() => void execute("input", { text: "\u0003" })}>Interrupt terminal command</Button><Button onClick={() => void execute("stop")}>Stop terminal</Button></>}<pre ref={element} className="terminal-output" aria-label="Terminal output">{status?.text}</pre><form onSubmit={e => { e.preventDefault(); const submitted = line; void execute("input", { text: `${submitted}\n` }).then(result => { if (result) setLine(current => current === submitted ? "" : current); }); }}><label>Terminal command<input value={line} onChange={e => setLine(e.target.value)} disabled={!status?.open} /></label><Button type="submit" disabled={!status?.open}>Send to terminal</Button></form><p>This terminal runs with the host account's permissions in the isolated project. Its private shell skips login configuration. Closing the pane leaves it running; Stop terminates its owned process group.</p></section>;
}
