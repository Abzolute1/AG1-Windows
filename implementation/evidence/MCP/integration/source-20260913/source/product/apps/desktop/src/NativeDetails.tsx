import { X } from "lucide-react";
import { useState } from "react";
import type { ClientDetails, ClientMessage, Command, JsonValue } from "./generated/protocol";
import { RecoveryControls } from "./RecoveryControls";
import { DetailPages } from "./DetailPages";
import { SessionHistory } from "./SessionHistory";
import { PlansPane } from "./PlansPane";
import { ResourcesPane } from "./ResourcesPane";
import { RuntimePane } from "./RuntimePane";
import { Button } from "./components/ui/button";
export function Blocks({ message }: { message: ClientMessage }) {
  return <>{message.blocks.map((block, i) => {
    switch (block.kind) {
      case "text": return <pre key={i}>{block.text}</pre>;
      case "thinking": return <details key={i}><summary>Reasoning</summary><pre>{block.text}</pre></details>;
      case "tool": return <details key={i}><summary>{block.name}</summary><pre>{JSON.stringify(block.arguments, null, 2)}</pre></details>;
      case "attachment": return <p key={i}>Attachment · {block.media_type} · {block.reference}</p>;
      case "unknown": return <details key={i}><summary>Preserved content</summary><pre>{JSON.stringify(block.value)}</pre></details>;
    }
  })}</>;
}
function Records({ value }: { value: JsonValue }) {
  if (value === null || value === undefined) return <p className="muted">Nothing recorded yet.</p>;
  if (typeof value !== "object") return <span>{String(value)}</span>;
  if (Array.isArray(value)) return value.length ? <ul>{value.map((v, i) => <li key={i}><Records value={v} /></li>)}</ul> : <p className="muted">No records.</p>;
  return <dl>{Object.entries(value).map(([key, v]) => <div key={key}><dt>{key.replaceAll("_", " ").replace(/([a-z])([A-Z])/g, "$1 $2")}</dt><dd><Records value={v} /></dd></div>)}</dl>;
}
export function NativeDetails({ details, status, model, run, onEditor, onClose, onArtifacts }: { details: ClientDetails; status: string; model: string; run: (command: Command) => Promise<JsonValue>; onEditor: (text: string) => void; onClose: () => void; onArtifacts: () => void }) {
  const [tab, setTab] = useState("Task"), [error, setError] = useState(""), [busy, setBusy] = useState(false);
  const [objective, setObjective] = useState(""), [contextTokens, setContextTokens] = useState(""), [contextFocus, setContextFocus] = useState("");
  const task = details.task as Record<string, JsonValue> | null;
  const idle = status === "idle";
  const execute = async (command: Command) => { if (busy) return; setBusy(true); setError(""); try { return await run(command); } catch (e) { setError(String(e)); return null; } finally { setBusy(false); } };
  return <aside className="details-drawer" aria-label="Task and runtime details">
    <header className="details-heading"><div><span>Conversation details</span><p>Plans, outputs and history</p></div><button aria-label="Close details" onClick={onClose}><X size={19} /></button></header>
    <nav aria-label="Details tabs">{["Task", "Plans", "Outputs", "History", "Advanced"].map(t => <button key={t} aria-pressed={tab === t} onClick={() => setTab(t)}>{t}</button>)}</nav>
    <div className="details-content">{error && <p className="inline-error" role="alert">{error}</p>}
    {tab === "Task" && <><h3>Current task</h3><p className="task-objective">{typeof task?.objective === "string" && task.objective ? task.objective : "No task has been set for this conversation."}</p>{task?.status && <span className="state-badge">{String(task.status)}</span>}<form onSubmit={e => { e.preventDefault(); if (idle && objective.trim()) void execute({ kind: "task", action: "set", args: { objective: objective.trim() } }); }}><label>Update objective<textarea rows={3} value={objective} placeholder="What should AG1 accomplish?" onChange={e => setObjective(e.target.value)} /></label><button className="outline-button" disabled={busy || !idle || !objective.trim()}>Save objective</button>{!idle && <p className="settings-footnote">You can edit the objective when this response finishes.</p>}</form><details className="advanced-records"><summary>Task record</summary><Records value={details.task} /><DetailPages section="task" run={run} /></details></>}
    {tab === "Outputs" && <><Button onClick={onArtifacts}>Open artifacts in the review panel</Button><details className="advanced-records"><summary>Project memory</summary><Records value={details.memory} /><DetailPages section="memory" run={run} /></details></>}
    {tab === "History" && <SessionHistory run={run} onEditor={onEditor} />}
    {tab === "Plans" && <PlansPane run={run} />}
    {tab === "Advanced" && <><p className="settings-footnote">Runtime inspection and maintenance for this conversation.</p><dl className="runtime-summary"><div><dt>Status</dt><dd>{status.replaceAll("_", " ")}</dd></div><div><dt>Model</dt><dd>{model}</dd></div><div><dt>Context limit</dt><dd>{String((details.context as Record<string, JsonValue>)?.working_context_limit ?? "Unavailable")}</dd></div></dl><details className="advanced-records"><summary>Context management</summary><Records value={details.context} /><DetailPages section="context" run={run} /><label>Working context token limit<input type="number" min="1024" value={contextTokens} onChange={e => setContextTokens(e.target.value)} /></label><Button disabled={busy || !idle || !contextTokens || Number(contextTokens) < 1024} onClick={() => void execute({ kind: "model", action: "context", args: { tokens: Number(contextTokens) } })}>Set context limit</Button><label>Preserve during compaction<input value={contextFocus} onChange={e => setContextFocus(e.target.value)} /></label><Button disabled={busy || !idle} onClick={() => void execute({ kind: "session", action: "compact", args: { focus: contextFocus || null } })}>Compact context</Button></details><details className="advanced-records"><summary>Activity & recovery</summary><Records value={details.activity} /><DetailPages section="activity" run={run} /><RecoveryControls run={run} /></details><details className="advanced-records"><summary>Runtime</summary><RuntimePane run={run} /></details><details className="advanced-records"><summary>Resources & plugins</summary><ResourcesPane run={run} /></details></>}
    </div>
  </aside>;
}
