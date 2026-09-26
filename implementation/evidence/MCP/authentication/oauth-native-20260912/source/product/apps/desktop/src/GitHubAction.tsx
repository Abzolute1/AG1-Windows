import { useEffect, useRef, useState, type ReactNode } from "react";
import { api, errorMessage } from "./bridge";
import { Source } from "./GitHubWorkbench";
export type GitHubRequest = { operation: string; repo?: string; parameters?: Record<string, unknown>; body?: Record<string, unknown> };
/** Shared presentation for explicit workflow, release and inbox mutations. */
export function GitHubAction({ title, scope, request, children, reviewLabel = "Review change" }: { title: string; scope: string; request: () => GitHubRequest; children?: ReactNode; reviewLabel?: string }) {
  const [preview, setPreview] = useState<any>(null), [result, setResult] = useState<any>(null), [error, setError] = useState(""), [busy, setBusy] = useState(false), [ack, setAck] = useState(false), [expired, setExpired] = useState(false);
  const epoch = useRef(0), pending = useRef<string | null>(null), working = useRef(false), deadline = useRef(0);
  function dismiss() { const id = pending.current; pending.current = null; setPreview(null); setAck(false); if (id) void api.github("api_dismiss", { confirmation: id }).catch(() => {}); }
  useEffect(() => { ++epoch.current; setPreview(null); setResult(null); setError(""); setBusy(false); working.current = false; return () => { ++epoch.current; const id = pending.current; pending.current = null; if (id) void api.github("api_dismiss", { confirmation: id }).catch(() => {}); }; }, [scope]);
  useEffect(() => { if (!preview) return; const timer = setTimeout(() => setExpired(true), Math.max(0, deadline.current - Date.now())); return () => clearTimeout(timer); }, [preview]);
  async function run(confirm: boolean) {
    if (working.current || confirm && (!pending.current || expired || Date.now() >= deadline.current || preview?.risk === "consequential" && !ack)) return;
    working.current = true; setBusy(true); setError(""); setResult(null); const ticket = epoch.current;
    try {
      const args = confirm ? { confirmation: pending.current } : request();
      if (confirm) { pending.current = null; setPreview(null); } else dismiss();
      const response = await api.github(confirm ? "api_confirm" : "api_preview", args);
      if (ticket !== epoch.current) { if (!confirm && response.ok && response.value?.confirmation) void api.github("api_dismiss", { confirmation: response.value.confirmation }).catch(() => {}); return; }
      if (!response.ok) throw new Error(`${response.error?.message ?? "GitHub change failed."}${response.error?.outcome === "unknown" ? " The change may have applied. Inspect GitHub before trying again." : ""}`);
      if (confirm) setResult(response.value);
      else {
        const p = response.value; pending.current = typeof p?.confirmation === "string" ? p.confirmation : null;
        if (!pending.current || p.account?.host !== "github.com" || typeof p.account?.login !== "string" || !["write", "consequential"].includes(p.risk) || !p.request || !Number.isFinite(p.expires_in_seconds) || p.expires_in_seconds <= 0 || p.expires_in_seconds > 300) { dismiss(); throw new Error("Invalid native GitHub preview."); }
        deadline.current = Date.now() + p.expires_in_seconds * 1000; setPreview(p); setAck(false); setExpired(false);
      }
    } catch (e) { if (ticket === epoch.current) setError(errorMessage(e)); }
    finally { if (ticket === epoch.current) { working.current = false; setBusy(false); } }
  }
  return <section className="settings-card github-action" aria-label={title}><h4>{title}</h4>
    {!preview ? <form onSubmit={e => { e.preventDefault(); void run(false); }}><fieldset disabled={busy} style={{ border: 0, margin: 0, padding: 0, minWidth: 0 }}>{children}<button className="outline-button" type="submit">{reviewLabel}</button></fieldset></form> : <><p>Account: @{preview.account.login} · github.com</p><p>{preview.warning}</p><Source value={preview.request} label="Reviewed GitHub change" />{expired && <p role="status">This preview expired. Review the change again.</p>}{preview.risk === "consequential" && <label className="checkbox-label"><input type="checkbox" disabled={busy} checked={ack} onChange={e => setAck(e.target.checked)} />I approve this exact target and its potential costs or destructive effects.</label>}<div className="github-actions"><button className="outline-button" disabled={busy} onClick={dismiss}>Back to change form</button><button className="outline-button" disabled={busy || expired || preview.risk === "consequential" && !ack} onClick={() => void run(true)}>Confirm reviewed change</button></div></>}
    {busy && <><p role="status">Working with GitHub…</p><button className="outline-button" onClick={() => void api.github("api_cancel").catch(() => {})}>Cancel GitHub change</button></>}{error && <p role="alert">{error}</p>}
    {result && <><p role="status">GitHub accepted the request. Refresh the view to check its current state.{result.status === 202 ? " Processing may continue in the background." : ""}</p><details><summary>Change response</summary><Source value={result.data} label="GitHub change response" /></details></>}
  </section>;
}
