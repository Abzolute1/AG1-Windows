import { useEffect, useRef, useState } from "react";
import type { Command, JsonValue } from "./generated/protocol";
import { reviewError } from "./reviewErrors";
import { UndoChange } from "./UndoChange";

type Run = (command: Command) => Promise<JsonValue>;
export type Change = { id: string; path: string; root: string; call_id: string; state: string; operation: string;
  before_sha256: string | null; after_sha256: string;
  summary: { addedLines: number; removedLines: number } | null };
type Page = { changes: Change[]; next: number | null; notice: string; turn: string };
function useTurnChanges(turn: string, revision: string, run: Run) {
  const [page, setPage] = useState<Page | null>(null), [busy, setBusy] = useState(false), [error, setError] = useState("");
  const epoch = useRef(0);
  async function load(offset = 0) {
    const ticket = ++epoch.current; setBusy(true); setError("");
    try {
      const value = await run({ kind: "files", action: "changes", args: { turn, offset } }) as unknown as Page;
      if (ticket !== epoch.current) return;
      if (!value || value.turn !== turn || !Array.isArray(value.changes) || value.changes.length > 64 ||
        value.next !== null && (!Number.isSafeInteger(value.next) || value.next <= offset) || value.changes.some(c =>
          !c || typeof c.id !== "string" || typeof c.path !== "string" || typeof c.root !== "string" ||
          !["prepared", "committed", "not_committed", "uncertain"].includes(c.state) ||
          c.summary && [c.summary.addedLines, c.summary.removedLines].some(n => !Number.isSafeInteger(n) || n < 0))) throw new Error("The native change list could not be verified. Refresh to try again.");
      setPage(old => ({ ...value, changes: (offset === 0 ? value.changes : [...new Map([...(old?.changes ?? []), ...value.changes].map(c => [c.id, c])).values()]).slice(0, 500) }));
    } catch (e) { if (ticket === epoch.current) setError(reviewError(e)); }
    finally { if (ticket === epoch.current) setBusy(false); }
  }
  useEffect(() => { setPage(null); void load(); return () => { epoch.current++; }; }, [turn, revision, run]);
  return { page, busy, error, load };
}

export function TurnChangeCard({ turn, revision, run, review }: { turn: string; revision: string; run: Run; review: (turn: string) => void }) {
  const { page, busy, error, load } = useTurnChanges(turn, revision, run);
  const committed = page?.changes.filter(c => c.state === "committed") ?? [];
  const uncertain = page?.changes.filter(c => c.state === "prepared" || c.state === "uncertain") ?? [];
  if (!error && !committed.length && !uncertain.length && page?.next == null) return null;
  const files = [...new Set(committed.map(c => `${c.root}/${c.path}`))];
  return <section className="turn-change-card" aria-label="Native changes for this turn">
    {error ? <p role="alert">Change review could not load. <button disabled={busy} onClick={() => void load()}>Retry</button><details><summary>Details</summary>{error}</details></p> : <>
      <strong>{files.length} {files.length === 1 ? "file" : "files"} changed{page?.next !== null ? " on loaded pages" : ""}</strong>
      {!!uncertain.length && <p>{uncertain.length} {uncertain.length === 1 ? "operation needs" : "operations need"} outcome review.</p>}
      <ul>{committed.slice(0, 3).map(c => <li key={c.id} title={`${c.root}/${c.path}`}><span>{c.path}</span>{c.summary && <small>+{c.summary.addedLines} −{c.summary.removedLines}</small>}</li>)}</ul>
      <button onClick={() => review(turn)}>Review turn changes</button>
      {page?.next != null && <button disabled={busy || page.changes.length >= 500} onClick={() => void load(page.next!)}>Load more changes</button>}
    </>}
  </section>;
}

type BytePage = { base64: string; next: string | null; bytes: number; sha256: string; side: string; id: string; unavailable?: string };
async function readPage(run: Run, id: string, side: string, offset: number) {
  const page = await run({ kind: "files", action: "change_read", args: { id, side, offset: String(offset) } }) as unknown as BytePage;
  if (typeof page?.unavailable === "string") return { page, bytes: new Uint8Array() };
  if (!page || page.id !== id || page.side !== side || !Number.isSafeInteger(page.bytes) || page.bytes < 0 ||
      typeof page.base64 !== "string" || page.base64.length > 32000 || !/^[a-f0-9]{64}$/.test(page.sha256)) throw new Error("Invalid native snapshot page");
  const bytes = Uint8Array.from(atob(page.base64), c => c.charCodeAt(0)), end = offset + bytes.length;
  if (bytes.length > 24000 || end > page.bytes || !bytes.length && end !== page.bytes ||
      (page.next === null ? end !== page.bytes : page.next !== String(end) || end >= page.bytes)) throw new Error("Incomplete native snapshot page");
  return { page, bytes };
}

function ChangeSource({ change, run, effect }: { change: Change; run: Run; effect?: Run }) {
  const [side, setSide] = useState("diff"), [offset, setOffset] = useState(0), [text, setText] = useState("");
  const [next, setNext] = useState<string | null>(null), [busy, setBusy] = useState(false), [error, setError] = useState(""), [url, setUrl] = useState("");
  const epoch = useRef(0);
  const decoder = useRef(new TextDecoder("utf-8", { fatal: true }));
  useEffect(() => { if (!url) return; return () => URL.revokeObjectURL(url); }, [url]);
  useEffect(() => {
    const ticket = ++epoch.current; setBusy(true); setError(""); setText(""); setUrl("");
    void readPage(run, change.id, side, offset).then(({ page, bytes }) => {
      if (ticket !== epoch.current) return;
      setNext(page.next ?? null);
      if (page.unavailable) { setText(page.unavailable); return; }
      if (!offset) decoder.current = new TextDecoder("utf-8", { fatal: true });
      try { setText(decoder.current.decode(bytes, { stream: page.next !== null }) || (page.bytes === 0 ? "Empty file" : "")); }
      catch { setText(`Byte view (hexadecimal)\n${Array.from(bytes, b => b.toString(16).padStart(2, "0")).join(" ")}`); }
    }).catch(e => { if (ticket === epoch.current) setError(reviewError(e)); }).finally(() => { if (ticket === epoch.current) setBusy(false); });
    return () => { epoch.current++; };
  }, [change.id, run, side, offset]);
  async function download() {
    const ticket = ++epoch.current; setBusy(true); setError("");
    try {
      const first = await readPage(run, change.id, side, 0);
      if (ticket !== epoch.current) return;
      if (first.page.unavailable) throw new Error(first.page.unavailable);
      if (first.page.bytes > 8 * 1024 * 1024) throw new Error("Downloads are limited to 8 MB. Review source in pages.");
      const bytes = new Uint8Array(first.page.bytes); bytes.set(first.bytes);
      let at = first.bytes.length;
      while (at < bytes.length) {
        const part = await readPage(run, change.id, side, at);
        if (ticket !== epoch.current) return;
        if (part.page.sha256 !== first.page.sha256 || part.page.bytes !== bytes.length || !part.bytes.length) throw new Error("Native snapshot changed during download");
        bytes.set(part.bytes, at); at += part.bytes.length;
      }
      const digest = Array.from(new Uint8Array(await crypto.subtle.digest("SHA-256", bytes)), b => b.toString(16).padStart(2, "0")).join("");
      if (digest !== first.page.sha256) throw new Error("Native snapshot integrity mismatch");
      if (ticket === epoch.current) setUrl(URL.createObjectURL(new Blob([bytes], { type: "application/octet-stream" })));
    } catch (e) { if (ticket === epoch.current) setError(reviewError(e)); }
    finally { if (ticket === epoch.current) setBusy(false); }
  }
  return <section className="review-diff" aria-label="Retained file change">
    <h3>{change.path}</h3><p className="review-caption">{change.state === "committed" ? "Recorded native change" : `Outcome: ${change.state}. After shows intended bytes, not proof of publication.`}</p>
    <nav className="artifact-actions" aria-label="Change source views">{["diff", "before", "after"].map(value => <button key={value} aria-pressed={side === value} onClick={() => { setSide(value); setOffset(0); }}>{value === "diff" ? "Diff" : value === "before" ? "Before" : "After"}</button>)}</nav>
    {busy && <p role="status">Loading change…</p>}{error && <p role="alert">{error}</p>}
    <pre tabIndex={0}>{text}</pre>
    <div className="artifact-actions">{!!offset && <button disabled={busy} onClick={() => setOffset(0)}>Start of source</button>}{next !== null && <button disabled={busy} onClick={() => setOffset(Number(next))}>Next source page</button>}
      {side !== "diff" && (url ? <a download={`${side}-${change.path.split(/[\\/]/).at(-1) || "file"}`} href={url}>Download exact {side}</a> : <button disabled={busy} onClick={() => void download()}>Prepare {side} download</button>)}
    </div><p className="review-caption">Retained bytes at the time of this operation. Source page begins at byte {offset}. The workspace may have changed since.</p>
    {effect && <UndoChange id={change.id} path={change.path} run={run} effect={effect} />}
  </section>;
}

export function TurnReview({ turn, run, effect }: { turn: string; run: Run; effect?: Run }) {
  const { page, busy, error, load } = useTurnChanges(turn, "review", run);
  const [selected, setSelected] = useState<Change | null>(null), [filter, setFilter] = useState("");
  useEffect(() => { setSelected(null); setFilter(""); }, [turn, run]);
  return <section className="workspace-review">
    <div className="review-toolbar"><span>Selected turn changes</span><button disabled={busy} onClick={() => void load()}>Refresh</button></div>
    <p className="review-caption">{page?.notice || "Loading native change receipts…"}</p>
    {error && <p role="alert">{error}</p>}{busy && <p role="status">Loading changes…</p>}
    <input className="review-filter" aria-label="Filter turn files" value={filter} placeholder="Filter changed files…" onChange={e => setFilter(e.target.value)} />
    <div className="review-file-list">{page?.changes.filter(c => `${c.path} ${c.root}`.toLowerCase().includes(filter.toLowerCase())).map(c => <button key={c.id} aria-pressed={selected?.id === c.id} onClick={() => setSelected(c)} title={`${c.root}/${c.path}`}><span>{c.path}</span><small>{c.state === "committed" ? c.summary ? `+${c.summary.addedLines} −${c.summary.removedLines}` : "Binary / large" : c.state}</small></button>)}</div>
    {!busy && page && !page.changes.length && <p>No native file changes are available for this turn.</p>}
    {page?.next != null && <button disabled={busy || page.changes.length >= 500} onClick={() => void load(page.next!)}>Load more changes</button>}
    {selected && <ChangeSource key={selected.id} change={selected} run={run} effect={effect} />}
  </section>;
}
