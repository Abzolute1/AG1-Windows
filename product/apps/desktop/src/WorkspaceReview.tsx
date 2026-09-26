import { useEffect, useMemo, useRef, useState } from "react";
import { RefreshCw } from "lucide-react";
import type { Command, JsonValue } from "./generated/protocol";
import { reviewError } from "./reviewErrors";

export const DIFF_FILES_PER_PAGE = 100, DIFF_ROWS_PER_PAGE = 300, DIFF_SOURCE_PAGE = 32768;

export type DiffFile = { header: string; text: string; additions: number; deletions: number };
// Display-only parsing: header text is never an authoritative filesystem path.
export function diffFiles(text: string): DiffFile[] {
  return text.split(/(?=^diff --git )/m).filter(part => part.startsWith("diff --git ")).map(part => {
    const lines = part.split("\n");
    let hunk = false, additions = 0, deletions = 0;
    for (const line of lines) {
      if (line.startsWith("@@ ")) hunk = true;
      else if (hunk && line.startsWith("+")) additions++;
      else if (hunk && line.startsWith("-")) deletions++;
    }
    return { header: lines[0].slice(11), text: part, additions, deletions };
  });
}

export function WorkspaceReview({ run, visible = true }: { run: (command: Command) => Promise<JsonValue>; visible?: boolean }) {
  const [text, setText] = useState(""), [filter, setFilter] = useState(""), [selected, setSelected] = useState<number | null>(null);
  const [loading, setLoading] = useState(false), [error, setError] = useState(""), [loaded, setLoaded] = useState(false);
  const [filePage, setFilePage] = useState(0), [rowPage, setRowPage] = useState(0), [sourcePage, setSourcePage] = useState(0);
  const ticket = useRef(0);
  async function load() {
    const current = ++ticket.current; setLoading(true); setError("");
    try {
      let result = await run({ kind: "files", action: "diff", args: { paged: true } }) as { view?: string; base64?: string; sha256?: string; bytes?: number; next?: string | null };
      if (current !== ticket.current) return;
      let view = result?.view;
      if (typeof view !== "string") {
        const total = result?.bytes, sha = result?.sha256;
        if (!Number.isSafeInteger(total) || total! < 0 || total! > 1024 * 1024 || !/^[a-f0-9]{64}$/.test(sha ?? "")) throw new Error("The workspace did not return a valid diff snapshot.");
        const bytes = new Uint8Array(total!); let offset = 0;
        while (true) {
          if (result.bytes !== total || result.sha256 !== sha || typeof result.base64 !== "string" || result.base64.length > 32000) throw new Error("Workspace diff changed while loading. Refresh to try again.");
          const chunk = Uint8Array.from(atob(result.base64), c => c.charCodeAt(0));
          const end = offset + chunk.length;
          if (end > bytes.length || !chunk.length && end !== bytes.length || (result.next === null ? end !== bytes.length : result.next !== String(end) || end >= bytes.length)) throw new Error("Incomplete workspace diff page.");
          bytes.set(chunk, offset); offset = end;
          if (result.next === null) break;
          result = await run({ kind: "files", action: "diff", args: { paged: true, offset: String(offset), sha256: sha! } }) as typeof result;
          if (current !== ticket.current) return;
        }
        const digest = Array.from(new Uint8Array(await crypto.subtle.digest("SHA-256", bytes)), b => b.toString(16).padStart(2, "0")).join("");
        if (current !== ticket.current) return;
        if (digest !== sha) throw new Error("Workspace diff integrity mismatch. Refresh to try again.");
        view = new TextDecoder().decode(bytes);
      }
      if (view.length > 1024 * 1024) throw new Error("This diff exceeds the review display limit.");
      setText(view); setSelected(null); setLoaded(true); setFilePage(0); setRowPage(0); setSourcePage(0);
    } catch (e) { if (current === ticket.current) setError(reviewError(e)); }
    finally { if (current === ticket.current) setLoading(false); }
  }
  useEffect(() => {
    setText(""); setFilter(""); setSelected(null); setLoaded(false); setError(""); setLoading(false);
    setFilePage(0); setRowPage(0); setSourcePage(0);
    return () => { ticket.current++; };
  }, [run]);
  useEffect(() => { if (visible && !loaded) void load(); }, [run, visible, loaded]);
  const files = useMemo(() => diffFiles(text), [text]);
  const shown = files.map((file, index) => ({ file, index })).filter(({ file }) => file.header.toLowerCase().includes(filter.toLowerCase()));
  const file = selected === null ? undefined : files[selected];
  const lines = useMemo(() => file?.text.split("\n") ?? [], [file]);
  return <section className="workspace-review">
    <div className="review-toolbar"><span>Workspace changes</span><button disabled={loading} aria-label="Refresh changes" onClick={() => void load()}><RefreshCw size={15} /></button></div>
    <p className="review-caption">Tracked changes against HEAD, including edits made outside this chat. This is not a last-turn record.</p>
    {loading && <p role="status">Loading changes…</p>}{error && <p role="alert" className="inline-error">{error}</p>}
    {!!files.length && <><input className="review-filter" aria-label="Filter changed files" placeholder="Filter changed files…" value={filter} onChange={e => { setFilter(e.target.value); setFilePage(0); }} />
      <div className="review-file-list" aria-label="Changed files">{shown.slice(filePage * DIFF_FILES_PER_PAGE, (filePage + 1) * DIFF_FILES_PER_PAGE).map(({ file, index }) => <button key={index} aria-pressed={selected === index} title={file.header} onClick={() => { setSelected(index); setRowPage(0); }}><span>{file.header}</span><small><ins>+{file.additions}</ins> <del>−{file.deletions}</del></small></button>)}</div>
      {shown.length > DIFF_FILES_PER_PAGE && <div className="artifact-actions"><button disabled={!filePage} onClick={() => setFilePage(p => p - 1)}>Previous files</button><span>File page {filePage + 1} of {Math.ceil(shown.length / DIFF_FILES_PER_PAGE)}</span><button disabled={(filePage + 1) * DIFF_FILES_PER_PAGE >= shown.length} onClick={() => setFilePage(p => p + 1)}>Next files</button></div>}
      {!shown.length && <p className="review-caption">No files match this filter.</p>}
      {file ? <section className="review-diff" aria-label="Selected file diff"><h3>{file.header}</h3><pre tabIndex={0}>{lines.slice(rowPage * DIFF_ROWS_PER_PAGE, (rowPage + 1) * DIFF_ROWS_PER_PAGE).map((line, i) => <span key={i} className={line.startsWith("+") ? "added" : line.startsWith("-") ? "removed" : line.startsWith("@@") ? "hunk" : ""}>{line}{"\n"}</span>)}</pre>
        {lines.length > DIFF_ROWS_PER_PAGE && <div className="artifact-actions"><button disabled={!rowPage} onClick={() => setRowPage(p => p - 1)}>Previous diff page</button><span>Diff page {rowPage + 1} of {Math.ceil(lines.length / DIFF_ROWS_PER_PAGE)}</span><button disabled={(rowPage + 1) * DIFF_ROWS_PER_PAGE >= lines.length} onClick={() => setRowPage(p => p + 1)}>Next diff page</button></div>}
      </section> : <p className="review-caption">Select a file to review its changes.</p>}</>}
    {loaded && !files.length && <p className="review-empty">No tracked changes against HEAD.</p>}
    {loaded && <details className="review-metadata"><summary>Workspace diff and status source</summary><pre tabIndex={0}>{text.slice(sourcePage * DIFF_SOURCE_PAGE, (sourcePage + 1) * DIFF_SOURCE_PAGE).replaceAll("\0", "\n")}</pre>
      {text.length > DIFF_SOURCE_PAGE && <div className="artifact-actions"><button disabled={!sourcePage} onClick={() => setSourcePage(p => p - 1)}>Previous source page</button><span>Source page {sourcePage + 1} of {Math.ceil(text.length / DIFF_SOURCE_PAGE)}</span><button disabled={(sourcePage + 1) * DIFF_SOURCE_PAGE >= text.length} onClick={() => setSourcePage(p => p + 1)}>Next source page</button></div>}
    </details>}
  </section>;
}
