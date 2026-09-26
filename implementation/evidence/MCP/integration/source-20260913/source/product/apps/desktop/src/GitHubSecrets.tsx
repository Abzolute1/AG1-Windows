import { useEffect, useRef, useState } from "react";
import { api, errorMessage } from "./bridge";
import { Source } from "./GitHubWorkbench";
const targets = ["actions_repository", "actions_environment", "actions_organization", "dependabot_repository", "dependabot_organization", "codespaces_repository", "codespaces_organization", "codespaces_user", "agents_repository", "agents_organization"];
export function GitHubSecrets({ repo, scope }: { repo?: string; scope: string }) {
  const [target, setTarget] = useState("actions_repository"), [organization, setOrganization] = useState(""), [environment, setEnvironment] = useState(""), [name, setName] = useState(""), [visibility, setVisibility] = useState("selected"), [ids, setIds] = useState("");
  const [preview, setPreview] = useState<any>(null), [result, setResult] = useState<any>(null), [busy, setBusy] = useState(false), [error, setError] = useState(""), [ack, setAck] = useState(false), [expired, setExpired] = useState(false), [page, setPage] = useState(1);
  const epoch = useRef(0), pending = useRef<string | null>(null), working = useRef(false), deadline = useRef(0);
  const org = target.endsWith("_organization"), user = target === "codespaces_user", env = target === "actions_environment";
  function dismiss() { const id = pending.current; pending.current = null; setPreview(null); setAck(false); if (id) void api.github("api_dismiss", { confirmation: id }).catch(() => {}); }
  useEffect(() => {
    ++epoch.current; setPreview(null); setResult(null); setBusy(false); setError(""); setPage(1); working.current = false;
    return () => { ++epoch.current; const id = pending.current; pending.current = null; if (id) void api.github("api_dismiss", { confirmation: id }).catch(() => {}); };
  }, [repo, scope, target, organization, environment]);
  useEffect(() => { if (!preview) return; const timer = setTimeout(() => setExpired(true), Math.max(0, deadline.current - Date.now())); return () => clearTimeout(timer); }, [preview]);
  function request(action: string, nextPage: number) {
    if (!org && !user && !repo) throw new Error("Select a repository first.");
    const parameters: Record<string, unknown> = { target, ...(org ? { organization } : {}), ...(env ? { environment } : {}) };
    if (action !== "list") parameters.name = name;
    if (["list", "repositories"].includes(action)) Object.assign(parameters, { page: nextPage, per_page: 30 });
    if (action === "set" && org) parameters.visibility = visibility;
    if (action === "set_repositories" || action === "set" && (user || org && visibility === "selected")) {
      const values = ids.trim() ? ids.split(",").map(s => Number(s.trim())) : [];
      if (values.length > 100 || values.some(n => !Number.isSafeInteger(n) || n <= 0) || new Set(values).size !== values.length) throw new Error("Enter up to 100 unique positive repository IDs, separated by commas.");
      parameters.selected_repository_ids = values;
    }
    return { operation: `secrets.${action}`, ...(!org && !user ? { repo } : {}), parameters };
  }
  async function run(action: string, nextPage = 1) {
    if (working.current || action === "confirm" && (!pending.current || !ack || expired || Date.now() >= deadline.current)) return;
    working.current = true; setBusy(true); setError(""); const ticket = epoch.current;
    try {
      const read = ["list", "get", "repositories"].includes(action);
      const args = action === "confirm" ? { confirmation: pending.current } : request(action, nextPage);
      if (action === "confirm") { pending.current = null; setPreview(null); } else dismiss();
      setResult(null);
      const response = await api.github(action === "confirm" ? "api_confirm" : read ? "api_read" : "api_preview", args);
      if (ticket !== epoch.current) { if (!read && action !== "confirm" && response.ok && response.value?.confirmation) void api.github("api_dismiss", { confirmation: response.value.confirmation }).catch(() => {}); return; }
      if (!response.ok) throw new Error(`${response.error?.message ?? "Secret operation failed."}${response.error?.outcome === "unknown" ? " The change may have applied. Inspect secret metadata before trying again." : ""}`);
      if (read || action === "confirm") { setResult({ ...response.value, action }); setPage(nextPage); }
      else {
        const p = response.value; pending.current = typeof p?.confirmation === "string" ? p.confirmation : null;
        if (!pending.current || p.account?.host !== "github.com" || typeof p.account?.login !== "string" || !p.request?.id?.startsWith("secrets.") || p.risk !== "consequential" || !Number.isFinite(p.expires_in_seconds) || p.expires_in_seconds <= 0 || p.expires_in_seconds > 300) { dismiss(); throw new Error("Invalid native secret preview."); }
        deadline.current = Date.now() + p.expires_in_seconds * 1000; setPreview(p); setAck(false); setExpired(false);
      }
    } catch (e) { if (ticket === epoch.current) setError(errorMessage(e)); }
    finally { if (ticket === epoch.current) { working.current = false; setBusy(false); } }
  }
  return <section className="settings-card github-secrets" aria-label="GitHub secrets">
    <h4>GitHub secrets</h4><p>Review the destination here. Enter the value in AG1’s native dialog after confirmation. Secret values are never returned to this screen or your conversation.</p>
    {!preview ? <><label>Secret target<select disabled={busy} value={target} onChange={e => setTarget(e.target.value)}>{targets.map(t => <option key={t} value={t}>{t.replaceAll("_", " ")}</option>)}</select></label>{org && <label>Organization<input disabled={busy} value={organization} maxLength={100} onChange={e => setOrganization(e.target.value)} /></label>}{env && <label>Environment<input disabled={busy} value={environment} maxLength={255} onChange={e => setEnvironment(e.target.value)} /></label>}{!org && !user && <p>Repository: {repo ?? "Select a repository first"}</p>}
      <label>Secret name<input disabled={busy} value={name} maxLength={255} placeholder="MY_SECRET" onChange={e => setName(e.target.value)} /></label>
      {org && <label>Secret visibility<select disabled={busy} value={visibility} onChange={e => setVisibility(e.target.value)}><option value="selected">Selected repositories</option><option value="private">Private repositories</option><option value="all">All repositories</option></select></label>}
      {(org || user) && <label>Repository IDs<input disabled={busy} value={ids} onChange={e => setIds(e.target.value)} placeholder="42, 73 — empty means no repositories" maxLength={1800} /></label>}
      <div className="github-actions"><button disabled={busy} onClick={() => void run("list")}>List secret metadata</button><button disabled={busy || !name} onClick={() => void run("get")}>Inspect secret metadata</button><button disabled={busy || !name} onClick={() => void run("set")}>Review secret destination</button><button disabled={busy || !name} onClick={() => void run("delete")}>Review secret deletion</button>{(org || user) && <><button disabled={busy || !name} onClick={() => void run("repositories")}>Inspect secret repositories</button><button disabled={busy || !name} onClick={() => void run("set_repositories")}>Review repository access</button></>}</div>
    </> : <><p>Account: @{preview.account.login} · github.com</p><Source value={preview.request} label="Reviewed secret destination" /><label className="checkbox-label"><input type="checkbox" disabled={busy} checked={ack} onChange={e => setAck(e.target.checked)} />I reviewed this secret’s exact destination and access.</label>{expired && <p role="status">This review expired. Review the destination again.</p>}<div className="github-actions"><button disabled={busy} onClick={dismiss}>Back to secret form</button><button disabled={busy || !ack || expired} onClick={() => void run("confirm")}>{preview.request.id === "secrets.set" ? "Open private secret entry" : "Confirm secret change"}</button></div></>}
    {busy && <><p role="status">Working with GitHub. If private entry is open, enter the value there or cancel.</p><button onClick={() => void api.github("api_cancel").catch(() => {})}>Cancel secret operation</button></>}
    {error && <p role="alert">{error}</p>}
    {result && <><p role="status">{result.data?.secret_saved ? "GitHub saved the secret. Its value is not returned." : result.outcome === "applied" ? "GitHub applied the reviewed change." : "Secret metadata received."}</p><Source value={result.data} label="Secret metadata" />{["list", "repositories"].includes(result.action) && <div className="github-pagination"><button disabled={busy || page === 1} onClick={() => void run(result.action, page - 1)}>Previous secret page</button><span>Page {page}</span><button disabled={busy || !result.next} onClick={() => void run(result.action, page + 1)}>Next secret page</button></div>}</>}
  </section>;
}
