import { useEffect, useRef, useState } from "react";
import { api, errorMessage } from "./bridge";
import { Source } from "./GitHubWorkbench";
import type { GitRun } from "./GitHubLocal";
export function GitHubUpload({ repo, release, workspace, run }: { repo: string; release: number; workspace?: string; run?: GitRun }) {
  const [path, setPath] = useState(""), [name, setName] = useState(""), [mime, setMime] = useState("application/octet-stream");
  const [preview, setPreview] = useState<any>(null), [result, setResult] = useState<any>(null), [busy, setBusy] = useState(false), [error, setError] = useState(""), [ack, setAck] = useState(false), [expired, setExpired] = useState(false);
  const epoch = useRef(0), pending = useRef<string | null>(null), working = useRef(false), deadline = useRef(0);
  function dismiss() { const id = pending.current; pending.current = null; setPreview(null); setAck(false); if (id) void api.github("api_dismiss", { confirmation: id }).catch(() => {}); }
  useEffect(() => { ++epoch.current; setPreview(null); setResult(null); setBusy(false); setError(""); setPath(""); setName(""); working.current = false; return () => { ++epoch.current; const id = pending.current; pending.current = null; if (id) void api.github("api_dismiss", { confirmation: id }).catch(() => {}); }; }, [repo, release, workspace, run]);
  useEffect(() => { if (!preview) return; const timer = setTimeout(() => setExpired(true), Math.max(0, deadline.current - Date.now())); return () => clearTimeout(timer); }, [preview]);
  async function execute(confirm: boolean) {
    if (working.current || !run || !workspace || confirm && (!pending.current || !ack || expired || Date.now() >= deadline.current)) return;
    working.current = true; setBusy(true); setError(""); const ticket = epoch.current;
    try {
      if (confirm) {
        const confirmation = pending.current; pending.current = null; setPreview(null);
        const response = await api.github("api_confirm", { confirmation });
        if (ticket !== epoch.current) return;
        if (!response.ok) throw new Error(`${response.error?.message ?? "Upload failed."}${response.error?.outcome === "unknown" ? " The upload may have left an asset on GitHub. Inspect release assets before trying again." : ""}`);
        setResult(response.value?.data);
      } else {
        dismiss(); setResult(null);
        const p: any = await run({ kind: "files", action: "github_upload_preview", args: { operation: "releases.upload_asset", repo, parameters: { release_id: release, folder: workspace, path, name, content_type: mime } } });
        if (ticket !== epoch.current) { if (typeof p?.confirmation === "string") void api.github("api_dismiss", { confirmation: p.confirmation }).catch(() => {}); return; }
        pending.current = typeof p?.confirmation === "string" ? p.confirmation : null;
        if (!pending.current || p.account?.host !== "github.com" || typeof p.account?.login !== "string" || p.request?.id !== "releases.upload_asset" || p.risk !== "consequential" || typeof p.upload_source?.path !== "string" || typeof p.upload_source?.sha256 !== "string" || !Number.isSafeInteger(p.upload_source?.bytes) || p.upload_source.bytes < 0 || p.upload_source.bytes > 64 * 1024 * 1024 || !Number.isFinite(p.expires_in_seconds) || p.expires_in_seconds <= 0 || p.expires_in_seconds > 300) { dismiss(); throw new Error("Invalid native upload preview."); }
        deadline.current = Date.now() + p.expires_in_seconds * 1000; setPreview(p); setAck(false); setExpired(false);
      }
    } catch (e) { if (ticket === epoch.current) setError(errorMessage(e)); }
    finally { if (ticket === epoch.current) { working.current = false; setBusy(false); } }
  }
  return <section className="settings-card github-item-actions github-upload" aria-label="Upload release asset">
    <h4>Upload a release asset</h4><p>{repo} · Release {release}</p>
    {!workspace || !run ? <p>Attach a project and grant access to the source folder to upload a file.</p> : !preview && <form onSubmit={e => { e.preventDefault(); void execute(false); }}>
      <p>Source folder: {workspace}. Uploads publish file contents to this release; maximum 64 MiB.</p>
      <label>File path<input disabled={busy} required maxLength={4096} value={path} onChange={e => setPath(e.target.value)} placeholder="dist/package.zip" /></label>
      <label>Asset filename<input disabled={busy} required maxLength={255} value={name} onChange={e => setName(e.target.value)} placeholder="package.zip" /></label>
      <label>Media type<input disabled={busy} required maxLength={100} value={mime} onChange={e => setMime(e.target.value)} /></label>
      <button className="outline-button" disabled={busy} type="submit">Review file upload</button>
    </form>}
    {preview && <><p>Account: @{preview.account.login} · github.com</p><p>Asset filename: {typeof preview.request?.endpoint === "string" ? preview.request.endpoint.split("?name=")[1] : "Inspect the exact destination below"}</p><p>The native preview identifies the exact file bytes. Existing assets are not replaced or deleted automatically.</p><Source value={preview.upload_source} label="Reviewed upload file" /><details><summary>Exact upload destination</summary><Source value={preview.request} label="Exact upload request" /></details><label className="checkbox-label"><input type="checkbox" checked={ack} disabled={busy || expired} onChange={e => setAck(e.target.checked)} />I approve publishing these file bytes to this release.</label>{expired && <p role="alert">Upload preview expired. Review the file again.</p>}<div className="github-actions"><button disabled={busy} onClick={dismiss}>Back to upload form</button><button disabled={busy || expired || !ack} onClick={() => void execute(true)}>Confirm file upload</button></div></>}
    {busy && <><p role="status">Preparing or uploading the selected file…</p><button onClick={() => void api.github("api_cancel").catch(() => {})}>Cancel upload</button></>}
    {result && <><p role="status">GitHub accepted the upload. Refresh release assets to inspect it.</p><Source value={result} label="Upload receipt" /></>}
    {error && <p role="alert">{error}</p>}
  </section>;
}
