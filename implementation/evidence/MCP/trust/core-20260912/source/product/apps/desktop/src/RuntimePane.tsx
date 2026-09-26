import { lazy, Suspense, useEffect, useState } from "react";
import type { Command, JsonValue } from "./generated/protocol";
import { Button } from "./components/ui/button";
const TerminalPane = lazy(() => import("./TerminalPane"));
type Target = { id: string; kind: string; lifecycle: string; controls: Record<string, { available: boolean; reason?: string }> };
type Operation = { id: string; action: string; lifecycle: string; diagnostics: { severity: string; message: string; file?: string; line?: number }[]; logs: string[] };
type Runtime = { available: boolean; reason?: string; targets: Target[]; history: Operation[]; revision: string; preview: { available: boolean; reason?: string } };
export function RuntimePane({ run }: { run: (command: Command) => Promise<JsonValue> }) {
  const [runtime, setRuntime] = useState<Runtime | null>(null), [error, setError] = useState(""), [log, setLog] = useState("");
  const [pollError, setPollError] = useState("");
  const [terminal, setTerminal] = useState(false);
  const [position, setPosition] = useState({ operation: "", offset: "0", stream: 0 });
  useEffect(() => {
    let cancelled = false, inflight = false;
    const poll = async () => { if (inflight) return; inflight = true; try { const result = await run({ kind: "project", action: "status", args: {} }); if (!cancelled) { setRuntime(result as unknown as Runtime); setPollError(""); } } catch (e) { if (!cancelled) setPollError(String(e)); } finally { inflight = false; } };
    void poll(); const timer = setInterval(poll, 500); return () => { cancelled = true; clearInterval(timer); };
  }, [run]);
  const execute = async (action: string, args: JsonValue) => { setError(""); try { return await run({ kind: "project", action, args }); } catch (e) { setError(String(e)); return null; } };
  return <section aria-label="Project runtime"><h3>Project runtime</h3>{(error || pollError) && <p role="alert">{error || pollError}</p>}
    {!runtime ? <p>Loading runtime capabilities…</p> : !runtime.available ? <p>{runtime.reason}</p> : <>
      {runtime.targets.length === 0 && <p>No target manifest found. Create or open a web, Rust, Tauri, native or Godot project.</p>}
      {runtime.targets.map(target => <article key={target.id}><h4>{target.kind} · {target.lifecycle}</h4>{Object.entries(target.controls).map(([action, capability]) => <Button key={action} disabled={!capability.available} title={capability.reason} onClick={() => void execute("start", { target: target.id, operation: action, revision: runtime.revision })}>{action}</Button>)}<Button onClick={() => void execute("stop", {})}>Stop</Button></article>)}
      <Button onClick={() => void execute("discover", {})}>Refresh target discovery</Button><h4>Owned project preview</h4><Button disabled={!runtime.preview?.available} title={runtime.preview?.reason} onClick={() => void execute("preview_open", {})}>Open preview</Button><Button disabled={!runtime.preview?.available} onClick={() => void execute("preview_focus", {})}>Focus preview</Button><Button onClick={() => void execute("preview_close", {})}>Close preview</Button>{!runtime.preview?.available && <p>{runtime.preview?.reason}</p>}
      <h4>Problems and history</h4>{runtime.history.slice(0,30).map(operation => <details key={operation.id}><summary>{operation.action} · {operation.lifecycle}</summary>{operation.diagnostics.map((d, i) => <p key={i}>{d.severity}: {d.message} {d.file && `${d.file}:${d.line ?? ""}`}</p>)}<Button onClick={() => { setPosition({ operation: operation.id, offset: "0", stream: 0 }); setLog(""); }}>Open logs</Button></details>)}
      {position.operation && <><label>Log stream<select value={position.stream} onChange={e => { setPosition({ ...position, stream: Number(e.target.value), offset: "0" }); setLog(""); }}>{runtime.history.find(o => o.id === position.operation)?.logs.map((_, i) => <option key={i} value={i}>Command {i + 1}</option>)}</select></label><Button onClick={() => void execute("log", { ...position }).then(result => { if (result) { const page = result as { text: string; next: string }; setLog(page.text); setPosition({ ...position, offset: page.next }); } })}>Next log byte page</Button><pre className="artifact-page">{log}</pre></>}
    </>}
    <Button onClick={() => setTerminal(!terminal)}>{terminal ? "Hide terminal" : "Show terminal"}</Button>{terminal && <Suspense fallback={<p>Loading terminal…</p>}><TerminalPane run={run} /></Suspense>}
  </section>;
}
