import { useEffect, useRef, useState } from "react";
import { api, errorMessage } from "./bridge";
import { GitHubAction } from "./GitHubAction";
import { Source } from "./GitHubWorkbench";
export function GitHubWorkflow({ repo, kind, id, scope }: { repo: string; kind: "workflow" | "run" | "job"; id: number; scope: string }) {
  const [detail, setDetail] = useState<any>(null), [error, setError] = useState(""), [busy, setBusy] = useState(false), [revision, setRevision] = useState(0);
  const [ref, setRef] = useState(""), [inputs, setInputs] = useState("{}"), [action, setAction] = useState("rerun");
  const epoch = useRef(0);
  async function read() {
    const ticket = ++epoch.current; setBusy(true); setError(""); setDetail(null); setRevision(v => v + 1);
    try {
      const response = await api.github("api_read", { operation: `actions.${kind}`, repo, parameters: { [`${kind}_id`]: id } });
      if (ticket !== epoch.current) return;
      if (!response.ok) throw new Error(response.error?.message ?? "Workflow detail failed.");
      const data = response.value?.data;
      if (!data || data.id !== id || kind !== "workflow" && typeof data.status !== "string") throw new Error("Invalid workflow detail target.");
      setDetail(data);
    } catch (e) { if (ticket === epoch.current) setError(errorMessage(e)); }
    finally { if (ticket === epoch.current) setBusy(false); }
  }
  useEffect(() => { setRef(""); setInputs("{}"); void read(); return () => { ++epoch.current; }; }, [repo, kind, id, scope]);
  const actionScope = `${scope}:${repo}:${kind}:${id}:${revision}`;
  return <section className="github-workflow" aria-label="Workflow details"><h4>{kind === "workflow" ? "Workflow" : kind === "run" ? "Run" : "Job"} #{id}</h4>
    <button className="outline-button" disabled={busy} onClick={() => void read()}>Refresh workflow detail</button>{busy && <p role="status">Loading current workflow details…</p>}{error && <p role="alert">{error}</p>}
    {detail && <><p>{String(detail.display_title ?? detail.name ?? "").slice(0, 300)} · {String(detail.status ?? detail.state ?? "")}{detail.conclusion ? ` · ${String(detail.conclusion)}` : ""}</p>{detail.head_branch && <p>Branch: {String(detail.head_branch).slice(0, 300)} · Commit: {String(detail.head_sha ?? "").slice(0, 64)}</p>}{detail.event && <p>Event: {String(detail.event).slice(0, 100)} · Attempt: {String(detail.run_attempt ?? 1)} · Actor: {String(detail.actor?.login ?? "Unknown").slice(0, 100)}</p>}{kind === "workflow" && <p>Workflow file: {String(detail.path ?? "").slice(0, 1024)}</p>}
      {Array.isArray(detail.steps) && <><h5>Job steps</h5><ol>{detail.steps.slice(0, 100).map((s: any, i: number) => <li key={i}><strong>{String(s.name ?? "Step").slice(0, 300)}</strong> · {String(s.conclusion ?? s.status ?? "").slice(0, 100)}{s.started_at ? ` · ${String(s.started_at).slice(0, 40)}` : ""}</li>)}</ol>{detail.steps.length > 100 && <p>Showing the first 100 steps. Complete details are available below.</p>}</>}
      {kind === "workflow" && <GitHubAction title="Dispatch workflow" scope={actionScope} reviewLabel="Review workflow dispatch" request={() => {
        if (!ref.trim() || ref.length > 255) throw new Error("Enter a branch or tag name.");
        const value: unknown = JSON.parse(inputs);
        if (!value || typeof value !== "object" || Array.isArray(value) || Object.keys(value).length > 25 || Object.values(value).some(v => !["string", "number", "boolean"].includes(typeof v) || typeof v === "number" && !Number.isFinite(v))) throw new Error("Inputs must be an object with up to 25 text, number or boolean values.");
        return { operation: "actions.dispatch", repo, parameters: { workflow_id: id }, body: { ref, inputs: value } };
      }}><p>This starts a workflow and may use paid runners or perform deployments. The workflow must support manual dispatch; GitHub checks its inputs and permissions.</p><label>Branch or tag<input required value={ref} maxLength={255} onChange={e => setRef(e.target.value)} /></label><label>Workflow inputs (JSON)<textarea rows={4} maxLength={60000} value={inputs} onChange={e => setInputs(e.target.value)} /></label></GitHubAction>}
      {kind === "run" && <GitHubAction title="Change workflow run" scope={actionScope} reviewLabel="Review run action" request={() => ({ operation: `actions.${action}`, repo, parameters: { run_id: id } })}><label>Run action<select value={action} onChange={e => setAction(e.target.value)}><option value="rerun">Run again</option><option value="cancel">Cancel run</option></select></label><p>Running again may use paid runners and repeat deployments. Cancellation is a request; refresh the run to see whether it has stopped.</p></GitHubAction>}
      <details><summary>Complete workflow detail</summary><Source value={detail} label="Workflow detail response" /></details>
    </>}
  </section>;
}
