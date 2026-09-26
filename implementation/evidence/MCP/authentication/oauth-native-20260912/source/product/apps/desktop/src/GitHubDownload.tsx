import { useEffect, useRef, useState } from "react";
import { api, errorMessage } from "./bridge";
const LIMIT = 64 * 1024 * 1024, PAGE = 256 * 1024;
type Receipt = { id: string; name: string; bytes: number; sha256: string; mime: string; operation: string };
function receipt(value: any): Receipt {
  if (!value || typeof value.id !== "string" || !/^download-[a-zA-Z0-9-]{1,110}$/.test(value.id) || !Number.isSafeInteger(value.bytes) || value.bytes < 0 || value.bytes > LIMIT || typeof value.name !== "string" || value.name.length > 255 || !/^[a-zA-Z0-9_.-]+$/.test(value.name) || !/^[a-f0-9]{64}$/.test(value.sha256) || typeof value.operation !== "string") throw new Error("Invalid native download receipt.");
  return value;
}
async function read(request: Record<string, unknown>) {
  const response = await api.github("api_read", request);
  if (!response.ok) throw new Error(response.error?.message ?? "GitHub download failed.");
  return response.value?.data;
}
export function GitHubDownload({ request, value, scope, label = "Download file" }: { request?: Record<string, unknown>; value?: unknown; scope: string; label?: string }) {
  const [retained, setRetained] = useState<Receipt | null>(null), [busy, setBusy] = useState(false), [error, setError] = useState(""), [progress, setProgress] = useState(0), [text, setText] = useState<string | null>(null);
  const epoch = useRef(0), working = useRef(false), urls = useRef(new Set<string>());
  const key = JSON.stringify(request ?? value);
  useEffect(() => {
    const clear = () => { ++epoch.current; working.current = false; setBusy(false); setRetained(null); setText(null); setProgress(0); setError(""); for (const url of urls.current) URL.revokeObjectURL(url); urls.current.clear(); };
    clear();
    if (value) { try { setRetained(receipt(value)); } catch { setError("Invalid native download receipt."); } }
    window.addEventListener("ag1-github-disconnected", clear);
    return () => { ++epoch.current; working.current = false; window.removeEventListener("ag1-github-disconnected", clear); for (const url of urls.current) URL.revokeObjectURL(url); urls.current.clear(); };
  }, [scope, key]);
  async function run(action: "fetch" | "save" | "preview") {
    if (working.current) return;
    working.current = true; setBusy(true); setError(""); setProgress(0); const ticket = epoch.current;
    const buffers: Uint8Array[] = [];
    try {
      if (action === "fetch") {
        const result = await read(request!);
        if (ticket === epoch.current) setRetained(receipt(result?.download));
        return;
      }
      if (!retained) return;
      const expected = retained;
      let offset = 0;
      do {
        const result = await read({ operation: "downloads.read", parameters: { id: expected.id, offset, length: action === "preview" ? 65536 : PAGE } });
        if (ticket !== epoch.current) return;
        const current = receipt(result?.download);
        if (current.id !== expected.id || current.sha256 !== expected.sha256 || current.bytes !== expected.bytes || result.offset !== offset || typeof result.base64 !== "string" || result.base64.length > PAGE * 2) throw new Error("Native download identity changed.");
        const raw = atob(result.base64), bytes = Uint8Array.from(raw, ch => ch.charCodeAt(0));
        if (bytes.length > PAGE || result.next_offset !== offset + bytes.length || result.next_offset > expected.bytes || result.complete !== (result.next_offset === expected.bytes) || (!bytes.length && !result.complete)) throw new Error("Invalid download byte page.");
        buffers.push(bytes); offset += bytes.length; setProgress(offset);
        if (action === "preview") { setText(new TextDecoder().decode(bytes)); return; }
      } while (offset < expected.bytes);
      const joined = new Uint8Array(expected.bytes); let at = 0; for (const bytes of buffers) { joined.set(bytes, at); at += bytes.length; }
      try {
        const digest = Array.from(new Uint8Array(await crypto.subtle.digest("SHA-256", joined)), b => b.toString(16).padStart(2, "0")).join("");
        if (digest !== expected.sha256) throw new Error("Downloaded bytes failed the digest check; nothing was saved.");
        if (ticket !== epoch.current) return;
        // The browser saves inert bytes only after this deliberate user action.
        const url = URL.createObjectURL(new Blob([joined], { type: "application/octet-stream" })); urls.current.add(url);
        const link = document.createElement("a"); link.href = url; link.download = expected.name; document.body.appendChild(link); link.click(); link.remove();
        setTimeout(() => { URL.revokeObjectURL(url); urls.current.delete(url); }, 1000);
      } finally { joined.fill(0); }
    } catch (e) { if (ticket === epoch.current) setError(errorMessage(e)); }
    finally { for (const bytes of buffers) bytes.fill(0); if (ticket === epoch.current) { working.current = false; setBusy(false); } }
  }
  return <section className="settings-card github-download" aria-label="GitHub download">
    {request && <button disabled={busy} onClick={() => void run("fetch")}>{retained ? "Download again" : label}</button>}
    {retained && <><p>{retained.name} · {retained.bytes.toLocaleString()} bytes</p><p>Available in this conversation for up to 15 minutes. Archives are saved without extraction.</p><button disabled={busy} onClick={() => void run("save")}>Save downloaded file</button>{retained.operation === "actions.download_job_log" && <button disabled={busy} onClick={() => void run("preview")}>Preview log text</button>}<details><summary>Download digest</summary><code style={{ overflowWrap: "anywhere" }}>{retained.sha256}</code></details></>}
    {busy && <><p role="status">{progress ? `${progress.toLocaleString()} bytes received…` : "Downloading…"}</p><button onClick={() => { ++epoch.current; working.current = false; setBusy(false); void api.github("api_cancel").catch(() => {}); }}>Cancel download</button></>}
    {text !== null && <><p>First 65,536 bytes, displayed as literal UTF-8 text; invalid characters are replaced.</p><pre style={{ whiteSpace: "pre-wrap", overflowWrap: "anywhere", maxHeight: 360, overflow: "auto" }}>{text}</pre><button onClick={() => setText(null)}>Close log preview</button></>}
    {error && <p role="alert">{error}</p>}
  </section>;
}
