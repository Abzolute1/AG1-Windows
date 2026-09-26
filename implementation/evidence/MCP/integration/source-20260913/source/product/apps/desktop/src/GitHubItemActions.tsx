import { useEffect, useRef, useState } from "react";
import { api, errorMessage } from "./bridge";
import { Source } from "./GitHubWorkbench";

type Item = { number: number; title?: string; body?: string | null; state?: string; merged?: boolean; head?: unknown };
type Action = "text" | "state" | "labels" | "assignees" | "milestone" | "reviewers";
type Preview = { confirmation: string; account: { login: string; host: string }; request: unknown; warning?: string; risk: string; expires_in_seconds: number };
const actions: [Action, string][] = [["text", "Edit title and description"], ["state", "Close or reopen"], ["labels", "Replace labels"], ["assignees", "Replace assignees"], ["milestone", "Set or remove milestone"], ["reviewers", "Request reviewers"]];
function names(text: string): string[] {
  const values = text.split("\n").map(v => v.trim()).filter(Boolean);
  if (values.length > 100 || values.some(v => v.length > 100) || new Set(values).size !== values.length) throw new Error("Use up to 100 distinct names, one per line, with at most 100 characters each.");
  return values;
}
export function GitHubItemMetadata({ item }: { item: any }) {
  function list(value: unknown, field: string) { return Array.isArray(value) ? value.slice(0, 100).map(v => typeof v === "string" ? v : v?.[field]).filter(v => typeof v === "string").join(", ") || "None reported" : "None reported"; }
  return <div className="settings-footnote"><p>Labels: {list(item.labels, "name")} · Assignees: {list(item.assignees, "login")} · Milestone: {typeof item.milestone?.title === "string" ? item.milestone.title : "None reported"}</p>{item.head && <p>Requested reviewers: {list(item.requested_reviewers, "login")} · Teams: {list(item.requested_teams, "slug")}</p>}<p>Up to 100 values per metadata list are shown.</p></div>;
}

// This form creates no authority. Only the native service's exact, one-use
// account/scope-bound preview can be consumed, including after a field edit.
export function GitHubItemActions({ repo, scope, item }: { repo: string; scope: string; item: Item }) {
  const [action, setAction] = useState<Action>("text"), [title, setTitle] = useState(item.title ?? ""), [body, setBody] = useState(item.body ?? "");
  const [state, setState] = useState("closed"), [values, setValues] = useState(""), [teams, setTeams] = useState(""), [milestone, setMilestone] = useState(""), [removeMilestone, setRemoveMilestone] = useState(false);
  const [preview, setPreview] = useState<Preview | null>(null), [busy, setBusy] = useState(false), [error, setError] = useState(""), [result, setResult] = useState<any>(null), [expired, setExpired] = useState(false), [ack, setAck] = useState(false);
  const pending = useRef<string | null>(null), epoch = useRef(0), working = useRef(false), deadline = useRef(0);
  const pull = !!item.head;
  function discard() {
    const token = pending.current; pending.current = null; deadline.current = 0;
    if (token) void api.github("api_dismiss", { confirmation: token }).catch(() => {});
    setPreview(null); setExpired(false); setAck(false);
  }
  useEffect(() => {
    ++epoch.current; working.current = false; setBusy(false); setError(""); setResult(null); setPreview(null); setExpired(false); setAck(false);
    setAction("text"); setTitle(item.title ?? ""); setBody(item.body ?? ""); setValues(""); setTeams(""); setMilestone(""); setRemoveMilestone(false); setState("closed");
    return () => { ++epoch.current; const token = pending.current; pending.current = null; deadline.current = 0; if (token) void api.github("api_dismiss", { confirmation: token }).catch(() => {}); };
  }, [repo, scope, item.number, item.title, item.body, item.state, item.merged, pull]);
  useEffect(() => {
    if (!preview) return;
    const timer = setTimeout(() => setExpired(true), Math.max(0, deadline.current - Date.now()));
    return () => clearTimeout(timer);
  }, [preview]);
  function request() {
    let operation = "issues.update", fields: Record<string, unknown>;
    if (action === "text") {
      if (!title.trim()) throw new Error("A title is required.");
      operation = pull ? "pulls.update" : "issues.update"; fields = { title, body };
    } else if (action === "state") {
      if (item.merged) throw new Error("A merged pull request cannot be reopened.");
      operation = pull ? "pulls.update" : "issues.update"; fields = { state };
    } else if (action === "labels" || action === "assignees") fields = { [action]: names(values) };
    else if (action === "milestone") {
      const n = Number(milestone);
      if (!removeMilestone && (!/^\d+$/.test(milestone) || !Number.isSafeInteger(n) || n < 1)) throw new Error("Enter a positive milestone number or explicitly choose removal.");
      fields = { milestone: removeMilestone ? null : n };
    } else {
      const reviewers = names(values), team_reviewers = names(teams);
      if (!pull || !reviewers.length && !team_reviewers.length) throw new Error("Enter at least one reviewer login or team slug.");
      operation = "pulls.reviewers"; fields = { reviewers, team_reviewers };
    }
    return { operation, repo, parameters: { number: item.number }, body: fields };
  }
  async function run(confirm: boolean) {
    if (working.current || confirm && (!pending.current || expired || Date.now() >= deadline.current || preview?.risk === "consequential" && !ack)) return;
    const ticket = epoch.current; working.current = true; setBusy(true); setError(""); setResult(null);
    try {
      const args = confirm ? { confirmation: pending.current } : request();
      if (confirm) { pending.current = null; setPreview(null); deadline.current = 0; }
      const reply = await api.github(confirm ? "api_confirm" : "api_preview", args);
      if (ticket !== epoch.current) { if (!confirm && typeof reply.value?.confirmation === "string") void api.github("api_dismiss", { confirmation: reply.value.confirmation }).catch(() => {}); return; }
      if (!reply.ok) throw new Error(`${reply.error?.message ?? "GitHub change failed."}${reply.error?.outcome === "unknown" ? " The change may have applied." : ""}`);
      if (confirm) setResult(reply.value);
      else {
        const p = reply.value;
        if (typeof p?.confirmation === "string") pending.current = p.confirmation;
        if (!pending.current || typeof p.account?.login !== "string" || p.account.host !== "github.com" || !p.request || !["write", "consequential"].includes(p.risk) || !Number.isFinite(p.expires_in_seconds) || p.expires_in_seconds <= 0 || p.expires_in_seconds > 300) { discard(); throw new Error("Invalid native GitHub preview. Review again."); }
        deadline.current = Date.now() + p.expires_in_seconds * 1000; setPreview(p); setExpired(false); setAck(false);
      }
    } catch (e) { if (ticket === epoch.current) setError(`${errorMessage(e)}${confirm ? " Inspect GitHub before repeating this change; it will not be retried automatically." : ""}`); }
    finally { if (ticket === epoch.current) { working.current = false; setBusy(false); } }
  }
  return <section className="github-item-actions settings-card" aria-label="Issue and pull request actions">
    <h4>Update {pull ? "pull request" : "issue"} #{item.number}</h4>
    <p>{repo} · Current displayed state: {item.merged ? "merged" : item.state ?? "unknown"}. Refresh the detail after a change.</p>
    {error && <p className="inline-error" role="alert">{error}</p>}
    {!preview && <form onSubmit={e => { e.preventDefault(); void run(false); }}>
      <label>Detail action<select aria-label="GitHub detail action" disabled={busy} value={action} onChange={e => { discard(); setAction(e.target.value as Action); setValues(""); setTeams(""); setResult(null); setError(""); }}>{actions.filter(([a]) => (pull || a !== "reviewers") && (!item.merged || a !== "state")).map(([a, label]) => <option key={a} value={a}>{label}</option>)}</select></label>
      {action === "text" && <><label>Updated title<input required maxLength={256} disabled={busy} value={title} onChange={e => setTitle(e.target.value)} /></label><label>Updated description<textarea rows={5} maxLength={60000} disabled={busy} value={body} onChange={e => setBody(e.target.value)} /></label></>}
      {action === "state" && <label>New item state<select disabled={busy} value={state} onChange={e => setState(e.target.value)}><option value="closed">Close</option><option value="open">Reopen</option></select></label>}
      {["labels", "assignees", "reviewers"].includes(action) && <label>{action === "labels" ? "Replacement labels" : action === "assignees" ? "Replacement assignee logins" : "Reviewer logins"}<textarea rows={3} maxLength={11000} disabled={busy} placeholder="One name per line" value={values} onChange={e => setValues(e.target.value)} /></label>}
      {action === "reviewers" && <label>Reviewer team slugs<textarea rows={3} maxLength={11000} disabled={busy} placeholder="One team slug per line" value={teams} onChange={e => setTeams(e.target.value)} /></label>}
      {action === "milestone" && <><label>Milestone number<input inputMode="numeric" maxLength={16} disabled={busy || removeMilestone} value={milestone} onChange={e => setMilestone(e.target.value)} /></label><label className="checkbox-label"><input type="checkbox" disabled={busy} checked={removeMilestone} onChange={e => setRemoveMilestone(e.target.checked)} />Remove the milestone</label></>}
      <p className="settings-footnote">Only the selected fields are sent. Text, state, labels, assignees and milestone updates replace their current GitHub values and can replace concurrent edits. Empty replacement lists remove all entries. GitHub still checks account permissions; a displayed state or successful check grants no write authority.</p>
      <button type="submit" disabled={busy} className="outline-button">Review item change</button>
    </form>}
    {preview && <><p>Account: @{preview.account.login} · {preview.account.host}</p><p>{preview.warning}</p><Source value={preview.request} label="Exact native item change" />{expired && <p role="alert">This preview expired. Return to the form and review again.</p>}{preview.risk === "consequential" && <label className="checkbox-label"><input type="checkbox" disabled={busy} checked={ack} onChange={e => setAck(e.target.checked)} />I reviewed the exact target and consequential effects.</label>}<div className="github-actions"><button type="button" disabled={busy} onClick={discard}>Back to item form</button><button type="button" className="primary-button" disabled={busy || expired || preview.risk === "consequential" && !ack} onClick={() => void run(true)}>Confirm item change</button></div></>}
    {busy && <p role="status">Working with GitHub…</p>}
    {result && <><p role="status">{result.outcome === "applied" ? "GitHub accepted the request." : "GitHub responded; inspect the returned state."} Refresh the detail to verify which fields changed; permissions may prevent some metadata updates.</p><details><summary>Item change response</summary><Source value={result.data} label="Item change result" /></details></>}
  </section>;
}
