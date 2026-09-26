import { createContext, useEffect, useMemo, useRef, useState } from "react";
import { api, errorMessage } from "./bridge";
import { GitHubInlineReview } from "./GitHubInlineReview";
import { Source } from "./GitHubWorkbench";
import { DIFF_ROWS_PER_PAGE } from "./WorkspaceReview";

export type GitHubReviewTarget = { repo: string; number: number };
export const GitHubReviewContext = createContext<{ open: (target: GitHubReviewTarget) => void; clear: () => void } | null>(null);
const text = (value: unknown, limit = 500) => typeof value === "string" ? value.slice(0, limit) : "";
const sha = (value: unknown): value is string => typeof value === "string" && /^[a-f0-9]{40}$/i.test(value);
type PatchLine = { text: string; old?: number; next?: number; kind: string };
export function pullPatchLines(patch: string): PatchLine[] {
  let old = 0, next = 0, hunk = false;
  return patch.split("\n").map(line => {
    const match = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/.exec(line);
    if (match) { old = Number(match[1]); next = Number(match[2]); hunk = true; return { text: line, kind: "hunk" }; }
    if (!hunk || line.startsWith("\\") || !line) return { text: line, kind: "" };
    if (line.startsWith("+")) return { text: line, next: next++, kind: "added" };
    if (line.startsWith("-")) return { text: line, old: old++, kind: "removed" };
    if (line.startsWith(" ")) return { text: line, old: old++, next: next++, kind: "" };
    return { text: line, kind: "" };
  });
}

export function GitHubPullReview({ target, visible = true }: { target: GitHubReviewTarget; visible?: boolean }) {
  const [data, setData] = useState<any>(null), [busy, setBusy] = useState(false), [error, setError] = useState("");
  const [filter, setFilter] = useState(""), [selected, setSelected] = useState<number | null>(null), [rowPage, setRowPage] = useState(0);
  const epoch = useRef(0), expected = useRef<{ head: string; base: string } | null>(null);
  async function load(page = 1, checkPage = 1, refresh = false) {
    const ticket = ++epoch.current; setBusy(true); setError(""); setData(null); setSelected(null); setRowPage(0);
    try {
      let identity = expected.current;
      if (refresh || !identity) {
        const initial = await api.github("api_read", { operation: "pulls.get", repo: target.repo, parameters: { number: target.number } });
        if (ticket !== epoch.current) return;
        if (!initial.ok) throw new Error(initial.error?.message ?? "Cannot inspect this pull request.");
        const item = initial.value?.data;
        if (item?.number !== target.number || !sha(item.head?.sha) || !sha(item.base?.sha)) throw new Error("GitHub returned an invalid pull request identity.");
        identity = { head: item.head.sha, base: item.base.sha };
      }
      const result = await api.github("pull_review", { repo: target.repo, number: target.number, head_sha: identity.head, base_sha: identity.base, page, check_page: checkPage });
      if (ticket !== epoch.current) return;
      if (result.head_sha !== identity.head || result.base_sha !== identity.base || !Array.isArray(result.files) || result.files.length > 30 || !Array.isArray(result.checks) || result.checks.length > 30 || result.page !== page || result.check_page !== checkPage) throw new Error("The native review page could not be verified.");
      if (result.files.some((f: any) => !f || typeof f.filename !== "string" || typeof f.patch === "string" && f.patch.length > 1024 * 1024)) throw new Error("Invalid or oversized GitHub patch.");
      expected.current = identity; setData(result);
    } catch (e) { if (ticket === epoch.current) setError(errorMessage(e)); }
    finally { if (ticket === epoch.current) setBusy(false); }
  }
  useEffect(() => {
    expected.current = null; setData(null); setError(""); setFilter(""); setSelected(null); setBusy(false);
    if (visible) void load(1, 1, true);
    return () => { ++epoch.current; };
  }, [target.repo, target.number, visible]);
  const files: any[] = data?.files ?? [], file = selected === null ? null : files[selected];
  const lines = useMemo(() => pullPatchLines(text(file?.patch, 1024 * 1024)), [file]);
  const checked = (data?.checks ?? []).filter((c: any) => c && typeof c === "object");
  return <section className="workspace-review" aria-label="Pull request review">
    <div className="review-toolbar"><strong>{target.repo} · PR #{target.number}</strong><button disabled={busy} onClick={() => void load(1, 1, true)}>Refresh PR review</button></div>
    <p className="review-caption">GitHub pull request changes, not local workspace changes or edits attributed to this chat. Reading this view does not approve a review or merge.</p>
    {busy && <p role="status">Loading exact PR head, files and checks…</p>}{error && <p className="inline-error" role="alert">{error}</p>}
    {data && <><h3>{text(data.item?.title)}</h3><p>{text(data.item?.state)}{data.item?.draft === true ? " · Draft" : ""} · Mergeability: {data.item?.mergeable === true ? "GitHub reports mergeable" : data.item?.mergeable === false ? "not mergeable" : "not yet known"}</p>
      <details className="review-metadata"><summary>Reviewed commits and limits</summary><p>Head: {data.head_sha}</p><p>Base: {data.base_sha}</p><p>{text(data.limits, 2000)}</p></details>
      <input className="review-filter" aria-label="Filter PR files" placeholder="Filter this file page…" value={filter} onChange={e => setFilter(e.target.value)} />
      <div className="review-file-list" aria-label="Pull request files">{files.map((f, i) => ({ f, i })).filter(({ f }) => f.filename.toLowerCase().includes(filter.toLowerCase())).map(({ f, i }) => <button key={i} aria-pressed={selected === i} onClick={() => { setSelected(i); setRowPage(0); }}><span>{f.filename}</span><small>{text(f.status, 40)} {Number.isSafeInteger(f.additions) && `+${f.additions}`} {Number.isSafeInteger(f.deletions) && `−${f.deletions}`}</small></button>)}</div>
      {!files.length && <p>No files on this page.</p>}
      <div className="artifact-actions"><button disabled={busy || data.page <= 1} onClick={() => void load(data.page - 1, data.check_page)}>Previous PR files</button><span>File page {data.page}</span><button disabled={busy || !data.files_next || data.page >= 100} onClick={() => void load(data.page + 1, data.check_page)}>Next PR files</button></div>
      {file && <section className="review-diff" aria-label="Selected PR file diff"><h3>{file.filename}</h3><GitHubInlineReview key={`${target.repo}:${target.number}:${data.head_sha}:${data.base_sha}:${file.filename}`} repo={target.repo} number={target.number} head={data.head_sha} base={data.base_sha} file={file} />{typeof file.previous_filename === "string" && <p>Previously: {file.previous_filename}</p>}{typeof file.patch === "string" && file.patch ? <><pre tabIndex={0}>{lines.slice(rowPage * DIFF_ROWS_PER_PAGE, (rowPage + 1) * DIFF_ROWS_PER_PAGE).map((line, i) => <span key={i} className={line.kind}><span className="pr-diff-gutter" aria-hidden="true">{String(line.old ?? "").padStart(6)} {String(line.next ?? "").padStart(6)} │ </span>{line.text}{"\n"}</span>)}</pre>{lines.length > DIFF_ROWS_PER_PAGE && <div className="artifact-actions"><button disabled={!rowPage} onClick={() => setRowPage(p => p - 1)}>Previous patch page</button><span>Patch page {rowPage + 1}</span><button disabled={(rowPage + 1) * DIFF_ROWS_PER_PAGE >= lines.length} onClick={() => setRowPage(p => p + 1)}>Next patch page</button></div>}</> : <p>Patch unavailable: binary, large, or omitted by GitHub. This is not an empty diff.</p>}</section>}
      <h3>Checks for the reviewed head</h3><p className="review-caption">These are check-run reports, not proof that every branch protection or required review is satisfied.</p>{checked.length ? <ul>{checked.map((c: any, i: number) => <li key={i}><strong>{text(c.name)}</strong> · {text(c.status, 80)}{c.conclusion ? ` · ${text(c.conclusion, 80)}` : " · no conclusion yet"}</li>)}</ul> : <p>No check runs on this page. This does not mean the PR is approved to merge.</p>}
      <div className="artifact-actions"><button disabled={busy || data.check_page <= 1} onClick={() => void load(data.page, data.check_page - 1)}>Previous PR checks</button><span>Check page {data.check_page}</span><button disabled={busy || !data.checks_next || data.check_page >= 100} onClick={() => void load(data.page, data.check_page + 1)}>Next PR checks</button></div>
      <details className="review-metadata"><summary>Exact PR page details</summary><Source value={data} label="GitHub PR page source" /></details>
    </>}
  </section>;
}
