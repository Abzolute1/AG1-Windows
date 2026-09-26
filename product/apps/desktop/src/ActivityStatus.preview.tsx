/** Development-only entry: /activity-preview.html on the Vite dev server.
 * Not imported by the application or included in the production entry. */
import { useState, type CSSProperties } from "react";
import { createRoot } from "react-dom/client";
import { ActivityStatus } from "./ActivityStatus";
import { TurnView } from "./NativeConversation";
import { WORKING_VERBS, type WorkingVerbKind } from "./workingVerbs";
import type { ClientMessage } from "./generated/protocol";
import "./dyad.css";
import "./app.css";
import "./redesign.css";
import "./activityPreview.css";

const kinds = Object.keys(WORKING_VERBS) as WorkingVerbKind[];
const run = async () => null;
const messages: ClientMessage[] = [
  { id: "preview-user", role: "user", blocks: [{ kind: "text", text: "Find the answer, then put the changes through their paces." }] },
  { id: "preview-assistant", role: "assistant", blocks: [{ kind: "text", text: "I’ve checked the sources. I’m working through the next step." }, { kind: "tool", name: "read", call_id: "read", arguments: { path: "notes.md" } }] },
  { id: "preview-result", role: "toolResult", tool: { name: "read", call_id: "read", failed: false }, blocks: [{ kind: "text", text: "Example notes." }] },
];
function Preview() {
  const [seed, setSeed] = useState(1), [cycle, setCycle] = useState(2400), [dark, setDark] = useState(true), [paused, setPaused] = useState(false), [status, setStatus] = useState("Thinking");
  return <main className="activity-preview" data-paused={paused || undefined} style={{ "--ag1-activity-cycle": `${cycle}ms` } as CSSProperties}>
    <header><h1>AG1 activity studio</h1><p>Simulated activity. No models, tools or native connection.</p></header>
    <div className="activity-preview-controls">
      <button onClick={() => setSeed(s => s + 1)}>Next verbs</button>
      <button onClick={() => { setDark(!dark); document.documentElement.classList.toggle("dark", !dark); document.documentElement.dataset.theme = !dark ? "dark" : "light"; }}>{dark ? "Light theme" : "Dark theme"}</button>
      <button aria-pressed={paused} onClick={() => setPaused(!paused)}>{paused ? "Resume motion" : "Pause motion"}</button>
      <label>Cycle <input type="range" min="1400" max="4400" step="100" value={cycle} onChange={e => setCycle(Number(e.target.value))} />{(cycle / 1000).toFixed(1)}s</label>
    </div>
    <div className="activity-preview-grid">{kinds.map(kind => <section key={kind}><small>{kind} · {WORKING_VERBS[kind].length} phrases</small><ActivityStatus activity={{ kind, label: kind, key: "preview" }} seed={String(seed)} /></section>)}</div>
    <section className="activity-preview-conversation">
      <label>Conversation phase <select value={status} onChange={e => setStatus(e.target.value)}>{["Thinking", "Generating", "Preparing request", "Waiting for model", "Queued · waiting for model", "Loading Qwen model", "Waiting for you", "Stopping response…"].map(s => <option key={s}>{s}</option>)}</select></label>
      <TurnView turn={{ id: `preview-${seed}`, messages }} active run={run} workTime="12s" workStatus={status} live={status === "Thinking" || status === "Generating" ? { id: "preview-live", committed: false, blocks: status === "Thinking" ? [{ kind: "thinking", text: "This is a simulated thought for checking the layout." }] : [{ kind: "text", text: "A response is arriving. The animated mark stays beneath the newest content." }] } : undefined} />
    </section>
    <footer>Edit ActivityMark.tsx for the shape, activityMotion.css for motion, and workingVerbs.ts for words. The application entry does not include this studio.</footer>
  </main>;
}
createRoot(document.getElementById("root")!).render(<Preview />);
