import { useState } from "react";
import type { ClientDetails, ClientMessage, Command, JsonValue } from "./generated/protocol";
import { ArtifactsPane } from "./ArtifactsPane";
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
export function NativeDetails({ details, status, model, run, onEditor }: { details: ClientDetails; status: string; model: string; run: (command: Command) => Promise<JsonValue>; onEditor: (text: string) => void }) {
  const [open, setOpen] = useState(false), [tab, setTab] = useState("Task"), [error, setError] = useState("");
  const [objective, setObjective] = useState("");
  const [contextTokens, setContextTokens] = useState("");
  const [providerModel, setProviderModel] = useState(""), [contextFocus, setContextFocus] = useState("");
  const execute = async (command: Command) => { setError(""); try { return await run(command); } catch (e) { setError(String(e)); return null; } };
  return <>
    <button className="context-strip" aria-expanded={open} onClick={() => setOpen(!open)}>
      <span>{status.replaceAll("_", " ")}</span><span>{model}</span><span>Context limit {String((details.context as Record<string, JsonValue>)?.working_context_limit ?? "unavailable")}</span><span>Task & activity {open ? "▴" : "▾"}</span>
    </button>
    {open && <aside className="details-drawer" aria-label="Task and runtime details">
      <nav aria-label="Details tabs">{["Task", "Plans", "Context", "Artifacts & memory", "Activity", "History", "Runtime", "Resources", "Models"].map(t => <button key={t} aria-pressed={tab === t} onClick={() => setTab(t)}>{t}</button>)}<button aria-label="Close details" onClick={() => setOpen(false)}>×</button></nav>
      {error && <p role="alert">{error}</p>}
      {tab === "Task" && <><Records value={details.task} /><DetailPages section="task" run={run} /><form onSubmit={e => { e.preventDefault(); void execute({ kind: "task", action: "set", args: { objective } }); }}><label>Objective<input value={objective} onChange={e => setObjective(e.target.value)} /></label><Button type="submit">Update objective</Button></form></>}
      {tab === "Context" && <><Records value={details.context} /><DetailPages section="context" run={run} /><label>Working context token limit<input type="number" min="1024" value={contextTokens} onChange={e => setContextTokens(e.target.value)} /></label><Button disabled={!contextTokens} onClick={() => void execute({ kind: "model", action: "context", args: { tokens: Number(contextTokens) } })}>Set context limit</Button><label>Preserve during compaction<input value={contextFocus} onChange={e => setContextFocus(e.target.value)} /></label><Button onClick={() => void execute({ kind: "session", action: "compact", args: { focus: contextFocus || null } })}>Compact context</Button></>}
      {tab === "Artifacts & memory" && <><ArtifactsPane run={run} /><h3>Project memory</h3><Records value={details.memory} /><DetailPages section="memory" run={run} /></>}
      {tab === "Activity" && <><Records value={details.activity} /><DetailPages section="activity" run={run} /><RecoveryControls run={run} /><p>Queues, retries, recovery and evidence retain their recorded states. Closing this drawer leaves host work running.</p></>}
      {tab === "History" && <SessionHistory run={run} onEditor={onEditor} />}
      {tab === "Plans" && <PlansPane run={run} />}
      {tab === "Resources" && <ResourcesPane run={run} />}
      {tab === "Runtime" && <RuntimePane run={run} />}
      {tab === "Models" && <><Records value={details.models} /><DetailPages section="models" run={run} /><label>Model<select value={providerModel} onChange={e => setProviderModel(e.target.value)}><option value="">Select a model</option>{details.models.map((value, i) => { const m = value as Record<string, JsonValue>; return <option key={i} value={`${m.provider}/${m.id}`} disabled={!!m.conflict}>{String(m.id)}{m.conflict ? " — configuration conflict" : ""}</option>; })}</select></label><Button disabled={!providerModel} onClick={() => { const [provider, ...id] = providerModel.split("/"); void execute({ kind: "model", action: "select", args: { provider, model_id: id.join("/") } }); }}>Use model</Button><p>Runtime selection is imported from the host. Tuning is a separate explicit operation.</p></>}
    </aside>}
  </>;
}
