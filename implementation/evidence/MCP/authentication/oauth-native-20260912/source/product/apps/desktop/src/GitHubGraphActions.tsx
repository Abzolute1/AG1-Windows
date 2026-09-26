import { useEffect, useRef, useState } from "react";
import { api, errorMessage } from "./bridge";
import { Source } from "./GitHubWorkbench";

export type GraphActionTarget = { kind: "project" | "discussion" | "comment" | "item" | "field"; id: string; projectId?: string; itemId?: string; discussionId?: string; title?: string; body?: string; closed?: boolean; archived?: boolean; isAnswer?: boolean };
type Action = "edit" | "state" | "comment" | "reply" | "answer" | "add" | "draft" | "archive" | "set" | "clear";
const options: Record<GraphActionTarget["kind"], [Action, string][]> = {
  project: [["edit", "Edit project"], ["state", "Close or reopen project"], ["add", "Add issue or pull request"], ["draft", "Add draft item"]],
  discussion: [["edit", "Edit discussion"], ["state", "Close or reopen discussion"], ["comment", "Add comment"]],
  comment: [["edit", "Edit comment"], ["reply", "Reply to comment"], ["answer", "Mark or unmark answer"]],
  item: [["archive", "Archive or unarchive item"]], field: [["set", "Set field value"], ["clear", "Clear field value"]],
};
type Preview = { confirmation: string; account: { login: string; host: string }; request: unknown; risk: string; expires_in_seconds: number };
export function GitHubGraphActions({ target, scope }: { target: GraphActionTarget; scope: string }) {
  const identity = JSON.stringify(target), first = options[target.kind][0][0];
  const [action, setAction] = useState<Action>(first), [title, setTitle] = useState(target.title ?? ""), [body, setBody] = useState(target.body ?? ""), [nodeId, setNodeId] = useState("");
  const [fieldType, setFieldType] = useState("text"), [fieldValue, setFieldValue] = useState(""), [state, setState] = useState("closed"), [reason, setReason] = useState("RESOLVED");
  const [preview, setPreview] = useState<Preview | null>(null), [error, setError] = useState(""), [result, setResult] = useState<any>(null), [busy, setBusy] = useState(false), [ack, setAck] = useState(false), [expired, setExpired] = useState(false);
  const epoch = useRef(0), pending = useRef<string | null>(null), deadline = useRef(0), working = useRef(false);
  function dismiss() { const token = pending.current; pending.current = null; deadline.current = 0; if (token) void api.github("api_dismiss", { confirmation: token }).catch(() => {}); setPreview(null); setAck(false); setExpired(false); }
  useEffect(() => {
    ++epoch.current; working.current = false; setBusy(false); setPreview(null); setError(""); setResult(null); setAction(first); setTitle(target.title ?? ""); setBody(target.body ?? ""); setNodeId(""); setFieldValue(""); setFieldType("text"); setState(target.closed ? "open" : "closed"); setAck(false); setExpired(false);
    return () => { ++epoch.current; const token = pending.current; pending.current = null; deadline.current = 0; if (token) void api.github("api_dismiss", { confirmation: token }).catch(() => {}); };
  }, [identity, scope]);
  useEffect(() => { if (!preview) return; const timer = setTimeout(() => setExpired(true), Math.max(0, deadline.current - Date.now())); return () => clearTimeout(timer); }, [preview]);
  function request() {
    const t = target; let operation: string, input: Record<string, unknown>;
    const required = (value: string | undefined, name: string) => { if (!value?.trim()) throw new Error(`${name} is required. Refresh the item if its identity is missing.`); return value; };
    if (t.kind === "project") {
      if (action === "add") { operation = "projects.add_item"; input = { projectId: t.id, contentId: required(nodeId, "Issue or pull request node ID") }; }
      else if (action === "draft") { operation = "projects.add_draft"; input = { projectId: t.id, title: required(title, "Title"), body }; }
      else { operation = "projects.update"; input = { projectId: t.id, ...(action === "state" ? { closed: state === "closed" } : { title: required(title, "Title"), readme: body }) }; }
    } else if (t.kind === "discussion") {
      if (action === "comment") { operation = "discussions.comment"; input = { discussionId: t.id, body: required(body, "Comment") }; }
      else if (action === "state") { operation = state === "closed" ? "discussions.close" : "discussions.reopen"; input = { discussionId: t.id, ...(state === "closed" ? { reason } : {}) }; }
      else { operation = "discussions.update"; input = { discussionId: t.id, title: required(title, "Title"), body }; }
    } else if (t.kind === "comment") {
      if (action === "reply") { operation = "discussions.comment"; input = { discussionId: required(t.discussionId, "Discussion ID"), replyToId: t.id, body: required(body, "Reply") }; }
      else if (action === "answer") { operation = t.isAnswer ? "discussions.unanswer" : "discussions.answer"; input = { id: t.id }; }
      else { operation = "discussions.edit_comment"; input = { commentId: t.id, body }; }
    } else if (t.kind === "item") { operation = t.archived ? "projects.unarchive" : "projects.archive"; input = { projectId: required(t.projectId, "Project ID"), itemId: t.id }; }
    else {
      operation = action === "clear" ? "projects.clear_field" : "projects.update_field";
      input = { projectId: required(t.projectId, "Project ID"), itemId: required(t.itemId, "Item ID"), fieldId: t.id };
      if (action === "set") {
        let value: unknown = fieldValue;
        if (fieldType === "number") { if (!fieldValue.trim() || !Number.isFinite(Number(fieldValue))) throw new Error("Enter a finite number."); value = Number(fieldValue); }
        else if (fieldType === "date" && !/^\d{4}-\d{2}-\d{2}$/.test(fieldValue)) throw new Error("Use a date in YYYY-MM-DD format.");
        else if (fieldType === "multiSelectOptionIds") { value = fieldValue.split("\n").map(v => v.trim()).filter(Boolean); if ((value as string[]).length > 100) throw new Error("Use at most 100 option IDs, one per line."); }
        else if (["singleSelectOptionId", "iterationId"].includes(fieldType)) required(fieldValue, "Option or iteration ID");
        input.value = { [fieldType]: value };
      }
    }
    return { operation, parameters: { input } };
  }
  async function run(confirm: boolean) {
    if (working.current || confirm && (!pending.current || expired || Date.now() >= deadline.current || preview?.risk === "consequential" && !ack)) return;
    working.current = true; setBusy(true); setError(""); setResult(null); const ticket = epoch.current;
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
        if (!pending.current || p.account?.host !== "github.com" || typeof p.account?.login !== "string" || !p.request || !["write", "consequential"].includes(p.risk) || !Number.isFinite(p.expires_in_seconds) || p.expires_in_seconds <= 0 || p.expires_in_seconds > 300) { dismiss(); throw new Error("Invalid native preview. Review again."); }
        deadline.current = Date.now() + p.expires_in_seconds * 1000; setPreview(p); setAck(false); setExpired(false);
      }
    } catch (e) { if (ticket === epoch.current) setError(`${errorMessage(e)}${confirm ? " Refresh GitHub before repeating this request." : ""}`); }
    finally { if (ticket === epoch.current) { working.current = false; setBusy(false); } }
  }
  const textAction = ["edit", "draft", "comment", "reply"].includes(action);
  return <section className="github-graph-actions github-item-actions settings-card" aria-label="Project and discussion actions">
    <h4>Change {target.kind}</h4><p>Target: {target.id}{target.projectId ? ` · Project ${target.projectId}` : ""}{target.itemId ? ` · Item ${target.itemId}` : ""}</p>
    {error && <p role="alert" className="inline-error">{error}</p>}
    {!preview && <form onSubmit={e => { e.preventDefault(); void run(false); }}>
      <label>Graph action<select aria-label="Graph action" disabled={busy} value={action} onChange={e => { dismiss(); setAction(e.target.value as Action); setError(""); setResult(null); setBody(["comment", "reply", "draft"].includes(e.target.value) ? "" : target.body ?? ""); setTitle(e.target.value === "draft" ? "" : target.title ?? ""); }}>{options[target.kind].map(([id, label]) => <option key={id} value={id}>{label}</option>)}</select></label>
      {textAction && <>{(["project", "discussion"].includes(target.kind) && ["edit", "draft"].includes(action)) && <label>Graph title<input required maxLength={256} disabled={busy} value={title} onChange={e => setTitle(e.target.value)} /></label>}<label>Graph text<textarea rows={4} maxLength={60000} disabled={busy} value={body} onChange={e => setBody(e.target.value)} /></label></>}
      {action === "state" && <><label>Graph state<select disabled={busy} value={state} onChange={e => setState(e.target.value)}><option value="closed">Close</option><option value="open">Reopen</option></select></label>{target.kind === "discussion" && state === "closed" && <label>Close reason<select disabled={busy} value={reason} onChange={e => setReason(e.target.value)}>{["RESOLVED", "OUTDATED", "DUPLICATE"].map(value => <option key={value}>{value}</option>)}</select></label>}</>}
      {action === "add" && <label>Issue or pull request node ID<input maxLength={1024} required disabled={busy} value={nodeId} onChange={e => setNodeId(e.target.value)} /></label>}
      {action === "answer" && <p>{target.isAnswer ? "Unmark this comment as the answer." : "Mark this comment as the answer."}</p>}
      {action === "archive" && <p>{target.archived ? "Unarchive this item." : "Archive this item."}</p>}
      {action === "clear" && <p>Clear this item's value for the selected field.</p>}
      {action === "set" && <><label>Field value type<select disabled={busy} value={fieldType} onChange={e => { setFieldType(e.target.value); setFieldValue(""); }}>{[["text", "Text"], ["number", "Number"], ["date", "Date"], ["singleSelectOptionId", "Single-select option ID"], ["multiSelectOptionIds", "Multi-select option IDs"], ["iterationId", "Iteration ID"]].map(([id, label]) => <option key={id} value={id}>{label}</option>)}</select></label><label>Field value<textarea rows={2} maxLength={60000} disabled={busy} value={fieldValue} onChange={e => setFieldValue(e.target.value)} /></label><p>Choose the field's configured type. Find option and iteration IDs in Project fields; use one multi-select ID per line.</p></>}
      <p className="settings-footnote">Account permissions apply. Replacement edits can replace concurrent changes. Review the exact target and fields before confirming, then refresh to verify the result.</p>
      <button type="submit" disabled={busy}>Review graph change</button>
    </form>}
    {preview && <><p>Account: @{preview.account.login} · {preview.account.host}</p><p>{String((preview.request as any)?.id ?? "Graph change").replaceAll(".", " · ").replaceAll("_", " ")}</p><Source value={(preview.request as any)?.body?.variables?.input} label="Reviewed graph fields" /><details><summary>Complete native graph request</summary><Source value={preview.request} label="Exact native graph change" /></details>{expired && <p role="alert">This preview expired. Review again.</p>}{preview.risk === "consequential" && <label className="checkbox-label"><input type="checkbox" disabled={busy} checked={ack} onChange={e => setAck(e.target.checked)} />I reviewed the exact target and consequential effects.</label>}<div className="github-actions"><button disabled={busy} onClick={dismiss}>Back to graph form</button><button disabled={busy || expired || preview.risk === "consequential" && !ack} onClick={() => void run(true)}>Confirm graph change</button></div></>}
    {busy && <><p role="status">Working with GitHub…</p><button onClick={() => void api.github("api_cancel").catch(e => setError(errorMessage(e)))}>Cancel graph request</button></>}
    {result && <><p role="status">GitHub responded. Refresh this view to verify the change.</p><details><summary>Graph change response</summary><Source value={result.data} label="Graph change result" /></details></>}
  </section>;
}
