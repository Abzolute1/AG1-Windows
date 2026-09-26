import { useEffect, useRef, useState } from "react";
import { Copy, FileText, RefreshCw } from "lucide-react";
import { artifactBytes, artifactFilename, artifactPage, artifactSize, artifactTitle, isRasterArtifact, MAX_ARTIFACT_DOWNLOAD, type Artifact, type ArtifactCatalog, type ArtifactRun } from "./artifactData";
import type { JsonValue } from "./generated/protocol";
import { reviewError } from "./reviewErrors";

export function ArtifactPreview({ artifact, run }: { artifact: Artifact; run: ArtifactRun }) {
  const [text, setText] = useState(""), [next, setNext] = useState<string | null>(null), [offset, setOffset] = useState(0);
  const [busy, setBusy] = useState(false), [error, setError] = useState(""), [url, setUrl] = useState("");
  const [loading, setLoading] = useState(true), [wrap, setWrap] = useState(true);
  const mounted = useRef(true), epoch = useRef(0), decoder = useRef(new TextDecoder()), downloadEpoch = useRef(0);
  const raster = isRasterArtifact(artifact), downloadable = artifact.bytes <= MAX_ARTIFACT_DOWNLOAD;
  useEffect(() => () => { if (url) URL.revokeObjectURL(url); }, [url]);
  async function read(at = 0) {
    const ticket = ++epoch.current; setLoading(true); setError("");
    try {
      const page = await artifactPage(run, artifact, at);
      if (!mounted.current || ticket !== epoch.current) return;
      if (at === 0) decoder.current = new TextDecoder();
      setText(decoder.current.decode(page.bytes, { stream: page.next !== null })); setOffset(at); setNext(page.next);
    } catch (e) { if (mounted.current && ticket === epoch.current) setError(reviewError(e)); }
    finally { if (mounted.current && ticket === epoch.current) setLoading(false); }
  }
  async function prepare() {
    if (!downloadable) return;
    const ticket = ++downloadEpoch.current; setBusy(true); setError("");
    const current = () => mounted.current && ticket === downloadEpoch.current;
    try {
      const bytes = await artifactBytes(run, artifact, current);
      if (current()) setUrl(URL.createObjectURL(new Blob([bytes], { type: raster ? artifact.mime_type : "application/octet-stream" })));
    } catch (e) { if (current()) setError(reviewError(e)); }
    finally { if (current()) setBusy(false); }
  }
  useEffect(() => { mounted.current = true; if (raster) { setLoading(false); void prepare(); } else void read(); return () => { mounted.current = false; epoch.current++; downloadEpoch.current++; }; }, [run]);
  return <section className="artifact-inspector" aria-label="Selected artifact">
    <header><h3>{artifactTitle(artifact)}</h3><span>{artifactSize(artifact.bytes)}</span></header>
    <div className="artifact-actions">
      {url ? <a download={artifactFilename(artifact)} href={url}>Download original</a> : <button disabled={busy || !downloadable} onClick={() => void prepare()}>{busy ? "Preparing…" : "Prepare download"}</button>}
      {!raster && <><button disabled={loading} onClick={() => void navigator.clipboard.writeText(text).catch(e => setError(String(e)))}><Copy size={13} />Copy displayed page</button><button aria-pressed={wrap} onClick={() => setWrap(!wrap)}>Wrap lines</button></>}
    </div>
    {!downloadable && <p className="review-caption">Downloads and image previews are limited to 8 MB. Text remains available in pages.</p>}
    {error && <p role="alert" className="inline-error">{error}<button onClick={() => void (raster ? prepare() : read(offset))}>Retry</button></p>}
    {raster ? url && <a className="artifact-image" href={url} download={artifactFilename(artifact)} title="Download original image"><img src={url} alt={artifactTitle(artifact)} /></a> : <>
      <p className="review-caption">{loading ? "Loading output…" : `Text page starting at byte ${offset.toLocaleString()}${next === null ? " · end of file" : ""}`}</p>
      <pre className={`artifact-source${wrap ? " wrap" : ""}`} tabIndex={0}>{text || (!loading ? "Empty file" : "")}</pre>
      <div className="artifact-actions">{offset > 0 && <button disabled={loading} onClick={() => void read(0)}>Start of file</button>}{next !== null && <button disabled={loading} onClick={() => void read(Number(next))}>Next page</button>}</div>
    </>}
    <details className="review-metadata"><summary>File information</summary><dl><dt>Record</dt><dd>{artifact.id}</dd><dt>Type</dt><dd>{artifact.mime_type || "Not specified"}</dd><dt>SHA-256</dt><dd>{artifact.sha256}</dd></dl></details>
  </section>;
}

export function ArtifactsPane({ run, visible = true, turn }: { run: ArtifactRun; visible?: boolean; turn?: string }) {
  const [catalog, setCatalog] = useState<ArtifactCatalog | null>(null), [selected, setSelected] = useState<Artifact | null>(null);
  const [filter, setFilter] = useState(""), [diagnostics, setDiagnostics] = useState(false), [error, setError] = useState(""), [busy, setBusy] = useState(false);
  const epoch = useRef(0);
  async function load(before: JsonValue = null) {
    const ticket = ++epoch.current; setError(""); setBusy(true);
    try {
      const value = await run({ kind: "artifacts", action: "list", args: { before, ...(turn ? { turn } : {}) } }) as unknown as ArtifactCatalog;
      if (ticket !== epoch.current) return;
      if (!value || !Array.isArray(value.artifacts) || value.artifacts.some(a => !a || typeof a.id !== "string" || typeof a.kind !== "string" || !Number.isSafeInteger(a.bytes) || a.bytes < 0 || !/^[a-f0-9]{64}$/.test(a.sha256))) throw new Error("Invalid artifact catalog");
      if (value.artifacts.length > 20) throw new Error("Invalid artifact catalog page size");
      setCatalog(old => ({ ...value, artifacts: (before === null ? value.artifacts : [...new Map([...(old?.artifacts ?? []), ...value.artifacts].map(a => [a.id, a])).values()]).slice(0, 500) }));
    } catch (e) { if (ticket === epoch.current) setError(reviewError(e)); }
    finally { if (ticket === epoch.current) setBusy(false); }
  }
  useEffect(() => {
    setCatalog(null); setSelected(null); setFilter(""); setDiagnostics(false); setError(""); setBusy(false);
    return () => { epoch.current++; };
  }, [run, turn]);
  useEffect(() => { if (visible && !catalog) void load(); }, [run, turn, visible, catalog]);
  const artifacts = catalog?.artifacts ?? [];
  const outputs = artifacts.filter(a => (diagnostics || !/^(context-|runtime-|provider-|request-|compaction-|native-change-|episode-draft-output$|tool-(?:arguments|rejected-arguments|native-result|original-result|post-hook-result|request-result)$)/.test(a.kind)) && `${artifactTitle(a)} ${a.id} ${a.mime_type ?? ""}`.toLowerCase().includes(filter.toLowerCase()));
  return <section className="artifacts-pane">
    <div className="review-toolbar"><span>{turn ? "Selected turn outputs" : "Conversation artifacts"}</span><button disabled={busy} aria-label="Refresh outputs" onClick={() => void load()}><RefreshCw size={15} /></button></div>
    <input className="review-filter" aria-label="Filter artifacts" placeholder="Filter artifacts…" value={filter} onChange={e => setFilter(e.target.value)} />
    {!turn && <label className="review-caption artifact-diagnostics"><input type="checkbox" checked={diagnostics} onChange={e => setDiagnostics(e.target.checked)} />Include diagnostic records</label>}
    {error && <p className="inline-error" role="alert">{error}</p>}{busy && <p role="status">Loading artifacts…</p>}
    <div className="artifact-list">{outputs.map(a => <button key={a.id} aria-pressed={selected?.id === a.id} onClick={() => setSelected(a)}><FileText size={16} /><span>{artifactTitle(a)}<small>{artifactSize(a.bytes)} · {new Date(a.created_at).toLocaleString()}</small></span></button>)}</div>
    {!busy && !outputs.length && <p className="review-empty">{filter ? "No loaded artifacts match this filter." : "No output artifacts on the loaded pages."}</p>}
    {catalog?.next != null && <button className="artifact-more" disabled={busy || artifacts.length >= 500} onClick={() => void load(catalog.next)}>Load earlier artifacts</button>}
    {artifacts.length >= 500 && <p className="review-caption">Showing up to 500 records. Refresh to return to recent artifacts.</p>}
    {selected ? <ArtifactPreview key={selected.id} artifact={selected} run={run} /> : !!outputs.length && <p className="review-caption">Select an artifact to preview or download it.</p>}
  </section>;
}
