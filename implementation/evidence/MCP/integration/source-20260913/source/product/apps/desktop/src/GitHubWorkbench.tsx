import { GitHubSecrets } from "./GitHubSecrets";
import { GitHubDownload } from "./GitHubDownload";
import { useEffect, useRef, useState } from "react";
import { api, errorMessage } from "./bridge";
type Operation = { id: string; area: string; method: string; path: string; risk: string };
type Preview = { account: { login: string; host: string }; request: unknown; confirmation: string; risk: string; warning: string; target_note?: string };
const PAGE = 16000;
export function Source({ value, label }: { value: unknown; label: string }) {
  const [page, setPage] = useState(0);
  const text = JSON.stringify(value, null, 2) ?? "";
  const pages = Math.max(1, Math.ceil(text.length / PAGE));
  useEffect(() => setPage(0), [text]);
  const current = Math.min(page, pages - 1);
  return <div className="github-api-source"><pre tabIndex={0} aria-label={label}>{text.slice(current * PAGE, (current + 1) * PAGE)}</pre>{pages > 1 && <div className="github-pagination"><button onClick={() => setPage(p => p - 1)} disabled={!current}>Previous source page</button><span>{current + 1} / {pages}</span><button onClick={() => setPage(p => p + 1)} disabled={current + 1 >= pages}>Next source page</button></div>}</div>;
}
function object(text: string) {
  const parsed: unknown = JSON.parse(text);
  if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) throw new Error("Parameters and body must be JSON objects.");
  return parsed;
}
function parametersFor(op: Operation, repo?: string) {
  if (op.id.startsWith("secrets.")) return JSON.stringify({ target: repo ? "actions_repository" : "actions_organization", ...(!repo ? { organization: "", ...(op.id === "secrets.set" ? { visibility: "selected", selected_repository_ids: [] } : {}) } : {}), ...(op.id !== "secrets.list" ? { name: "MY_SECRET" } : {}) }, null, 2);
  if (op.id === "downloads.read") return '{"id":"download-receipt","offset":0,"length":65536}';
  const params: Record<string, unknown> = {};
  for (const [, key] of op.path.matchAll(/\{([^}]+)\}/g)) if (key !== "repo") params[key] = "";
  if (op.id === "api.rest") Object.assign(params, { method: "GET", endpoint: "user/repos", page: 1, per_page: 30 });
  if (["discussions.list", "discussions.categories", "discussions.get"].includes(op.id)) { const [owner = "", name = ""] = repo?.split("/") ?? []; Object.assign(params, { owner, name }); if (op.id === "discussions.get") params.number = 1; }
  if (["projects.list", "projects.user_list"].includes(op.id)) params.login = "";
  if (["projects.items", "projects.get", "projects.fields", "projects.item_values", "discussions.replies"].includes(op.id)) params.id = "";
  if (op.id.startsWith("projects.field_")) Object.assign(params, { id: "", fieldId: "", fieldName: "" });
  if (op.path === "graphql" && ["write", "consequential"].includes(op.risk) && op.id !== "api.graphql") params.input = {};
  return JSON.stringify(params, null, 2);
}
export function GitHubWorkbench({ repo, scope }: { repo?: string; scope: string }) {
  const [operations, setOperations] = useState<Operation[]>([]), [operation, setOperation] = useState("");
  const [parameters, setParameters] = useState("{}"), [body, setBody] = useState("{}"), [preview, setPreview] = useState<Preview | null>(null);
  const [result, setResult] = useState<any>(null), [error, setError] = useState(""), [busy, setBusy] = useState(false), [acknowledged, setAcknowledged] = useState(false);
  const [schema, setSchema] = useState<any>(null), [schemaQuery, setSchemaQuery] = useState(""), [matches, setMatches] = useState<any>(null), [schemaPage, setSchemaPage] = useState(0);
  const epoch = useRef(0), pending = useRef<string | null>(null), working = useRef(false);
  const schemaRequest = useRef(0), searchRequest = useRef(0);
  const selected = operations.find(op => op.id === operation);
  const graphAccount = selected?.path === "graphql" && !["discussions.list", "discussions.categories", "discussions.get"].includes(operation);
  async function inspectSchema(id: string) { const ticket = epoch.current, query = ++schemaRequest.current; try { const value = await api.github("api_schema", { id }); if (ticket === epoch.current && query === schemaRequest.current) setSchema(value); } catch (e) { if (ticket === epoch.current && query === schemaRequest.current) setError(errorMessage(e)); } }
  async function searchSchema(page = 0) { const ticket = epoch.current, query = ++searchRequest.current; try { const value = await api.github("api_search", { query: schemaQuery, page }); if (ticket !== epoch.current || query !== searchRequest.current) return; if (!Array.isArray(value.items) || value.items.length > 40) throw new Error("Invalid schema search response."); setMatches(value); setSchemaPage(page); } catch (e) { if (ticket === epoch.current && query === searchRequest.current) setError(errorMessage(e)); } }
  function dismiss() { const token = pending.current; pending.current = null; if (token) void api.github("api_dismiss", { confirmation: token }).catch(() => {}); setPreview(null); setAcknowledged(false); }
  useEffect(() => {
    const ticket = ++epoch.current; setPreview(null); setResult(null); setError(""); setBusy(false); working.current = false;
    void api.github("capabilities").then(value => {
      if (ticket !== epoch.current) return;
      if (!Array.isArray(value.operations) || value.operations.length > 256 || value.operations.some((op: Operation) => !op || typeof op.id !== "string" || typeof op.path !== "string" || typeof op.area !== "string")) throw new Error("Invalid native GitHub catalog.");
      setOperations(value.operations); const op = value.operations.find((op: Operation) => op.id === (repo ? "repos.get" : "repos.list")) ?? value.operations[0];
      if (op) { setOperation(op.id); setParameters(parametersFor(op, repo)); setBody("{}"); }
    }).catch(e => { if (ticket === epoch.current) setError(errorMessage(e)); });
    return () => { ++epoch.current; const token = pending.current; pending.current = null; if (token) void api.github("api_dismiss", { confirmation: token }).catch(() => {}); };
  }, [repo, scope]);
  async function run(confirm: boolean) {
    if (working.current || !selected || (confirm && (!preview || (preview.risk === "consequential" && !acknowledged)))) return;
    working.current = true; setBusy(true); setError(""); const ticket = epoch.current;
    try {
      const params = object(parameters) as Record<string, unknown>;
      const request = { operation, ...(repo && !graphAccount && operation !== "downloads.read" && !(operation.startsWith("notifications.") && !selected.path.startsWith("repos/")) && !(operation.startsWith("secrets.") && !String(params.target).endsWith("_repository") && !String(params.target).endsWith("_environment")) ? { repo } : {}), parameters: params, ...(operation.startsWith("secrets.") || ["GET", "LOCAL"].includes(selected.method) || (operation === "api.rest" && params.method === "GET") || (selected.path === "graphql" && operation !== "api.graphql") ? {} : { body: object(body) }) };
      const confirmation = preview?.confirmation;
      // A consumed preview is never retried, even if the reply is interrupted.
      if (confirm) { pending.current = null; setPreview(null); setResult(null); }
      const reply = await api.github(confirm ? "api_confirm" : "api_preview", confirm ? { confirmation } : request);
      if (ticket !== epoch.current) {
        if (!confirm && reply.ok && reply.value?.confirmation) void api.github("api_dismiss", { confirmation: reply.value.confirmation }).catch(() => {});
        return;
      }
      if (!reply.ok) {
        const e = reply.error; setError(`${e?.message ?? "GitHub operation failed."}${e?.outcome === "unknown" ? " The change may have applied. Inspect GitHub before creating a new request." : ""}${e?.retry_after_seconds ? ` Retry guidance: wait ${e.retry_after_seconds} seconds.` : ""}${e?.retry_at_epoch_seconds ? ` Rate reset: ${new Date(e.retry_at_epoch_seconds * 1000).toLocaleString()}.` : ""}`); return;
      }
      if (confirm) setResult(reply.value);
      else {
        const p = reply.value;
        if (typeof p?.confirmation !== "string" || typeof p?.account?.login !== "string" || p.account.host !== "github.com") throw new Error("Invalid native GitHub preview.");
        pending.current = p.confirmation; setPreview(p); setAcknowledged(false); setResult(null);
      }
    } catch (e) { if (ticket === epoch.current) setError(`${errorMessage(e)}${confirm ? " Check GitHub before repeating the request." : ""}`); }
    finally { if (ticket === epoch.current) { working.current = false; setBusy(false); } }
  }
  return <section className="settings-card github-workbench" aria-label="GitHub API workbench">
    <h3>GitHub API workbench</h3><p>Shared native operations for AG1 and this panel. Account permissions still apply. No credentials or secret values belong in these fields.</p>
    <details><summary>Secret destinations and private entry</summary><GitHubSecrets repo={repo} scope={scope} /></details>
    <details><summary>Discover official API operations</summary><label>Search pinned API schemas<input aria-label="Search GitHub API schemas" maxLength={200} value={schemaQuery} onChange={e => { ++searchRequest.current; setSchemaQuery(e.target.value); setMatches(null); }} /></label><button onClick={() => void searchSchema()}>Search API schemas</button>{matches && <><p>{matches.total} matching operations</p><div className="github-schema-results">{matches.items.map((item: any) => <button key={item.id} onClick={() => void inspectSchema(item.id)}>{item.summary}<small>{item.method} {item.path}</small></button>)}</div><div className="github-pagination"><button disabled={!schemaPage} onClick={() => void searchSchema(schemaPage - 1)}>Previous schema page</button><button disabled={matches.next == null} onClick={() => void searchSchema(matches.next)}>Next schema page</button></div></>}</details>
    {error && <p role="alert" className="inline-error">{error}</p>}
    {!preview && <><label>Operation<select aria-label="GitHub operation" disabled={busy} value={operation} onChange={e => { const op = operations.find(op => op.id === e.target.value); if (!op) return; dismiss(); ++schemaRequest.current; setSchema(null); setOperation(op.id); setParameters(parametersFor(op, repo)); setBody(op.id === "api.graphql" ? '{"query":"query { viewer { login } }","variables":{}}' : "{}"); setResult(null); }}>{[...new Set(operations.map(op => op.area))].map(area => <optgroup key={area} label={area.replaceAll("_", " ")}>{operations.filter(op => op.area === area).map(op => <option key={op.id} value={op.id}>{op.id} · {op.risk}</option>)}</optgroup>)}</select></label>
    <p>{graphAccount ? "Account-wide GraphQL request" : repo || "Account-level request"} · {selected?.method} {selected?.path}</p>
    {graphAccount && <p>The selected repository does not constrain this request. Review the exact login, node IDs and query; node membership is validated by GitHub.</p>}
    {selected && <button onClick={() => void inspectSchema(selected.id)}>Inspect operation schema</button>}
    <label>Parameters (JSON)<textarea aria-label="GitHub parameters" rows={5} maxLength={32000} disabled={busy} value={parameters} onChange={e => setParameters(e.target.value)} /></label>
    {selected && !["GET", "LOCAL"].includes(selected.method) && (selected.path !== "graphql" || operation === "api.graphql") && <label>Body (JSON)<textarea aria-label="GitHub request body" rows={6} maxLength={64000} disabled={busy} value={body} onChange={e => setBody(e.target.value)} /></label>}
    {operation === "releases.upload_asset" && <p>Open Releases in the repository browser and select a release to upload a file from a granted folder.</p>}
    <p className="settings-footnote">Replace empty path parameters. REST list queries accept page/per_page (up to 100). GraphQL lists return pageInfo cursors; pass after for the next page. The operation’s official GitHub API schema defines body and input fields.</p>
    <button className="outline-button" disabled={busy || !selected || operation === "releases.upload_asset"} onClick={() => void run(false)}>Preview native request</button></>}
    {preview && <><p>Account: @{preview.account.login} · {preview.account.host}</p><p>{preview.warning}</p>{preview.target_note && <p>{preview.target_note}</p>}<Source value={preview.request} label="Exact GitHub request" />{preview.risk === "consequential" && <label className="checkbox-label"><input type="checkbox" disabled={busy} checked={acknowledged} onChange={e => setAcknowledged(e.target.checked)} />I reviewed the exact target, fields, and possible costs or destructive effects.</label>}<div className="github-actions"><button disabled={busy} onClick={dismiss}>Back to request</button><button className="primary-button" disabled={busy || (preview.risk === "consequential" && !acknowledged)} onClick={() => void run(true)}>{preview.risk === "read" ? "Run reviewed read" : "Confirm GitHub change"}</button></div></>}
    {busy && <div><p role="status">Working with the native GitHub service…</p><button onClick={() => void api.github("api_cancel").catch(e => setError(errorMessage(e)))}>Cancel GitHub request</button></div>}
    {schema && <details open><summary>Official request schema</summary><p>{schema.summary ?? schema.note}</p><Source value={schema} label="GitHub operation schema" /><button onClick={() => { ++schemaRequest.current; setSchema(null); }}>Close schema</button></details>}
    {result && <><p role="status">{result.outcome === "applied" ? "GitHub change applied." : "GitHub response received."} · HTTP {result.status}{result.rate_remaining != null ? ` · ${result.rate_remaining} requests remaining` : ""}</p>{result.next && <p>More results are available. Review the next page using the parameters above.</p>}{result.data?.download && !result.data?.base64 && <GitHubDownload value={result.data.download} scope={scope} />}<Source value={result.data} label="GitHub response" /></>}
    <p className="settings-footnote">GitHub.com only. Enterprise hosts, credential token management, automatic redirects, and automatic write retries are disabled. Local Git and binary transfers are separate capabilities.</p>
  </section>;
}
