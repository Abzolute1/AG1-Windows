import { useEffect, useRef, useState } from "react";
import { api, errorMessage } from "./bridge";
import type { GitHubRequest } from "./GitHubAction";

export function githubFailure(reply: any): string {
  const e = reply?.error;
  return `${e?.message ?? "GitHub request failed."}${e?.code ? ` (${e.code})` : ""}${e?.retry_after_seconds !== undefined && e?.retry_after_seconds !== null ? ` Retry after ${e.retry_after_seconds} seconds.` : ""}${e?.retry_at_epoch_seconds ? ` Retry time: ${new Date(e.retry_at_epoch_seconds * 1000).toLocaleString()}.` : ""}`;
}
/** Bounded, explicit reads; changing scope invalidates late responses. */
export function useGitHubRead(scope: string) {
  const [value, setValue] = useState<any>(null), [busy, setBusy] = useState(false), [error, setError] = useState("");
  const epoch = useRef(0);
  useEffect(() => { ++epoch.current; setValue(null); setError(""); setBusy(false); return () => { ++epoch.current; }; }, [scope]);
  async function read(request: GitHubRequest) {
    const ticket = ++epoch.current; setBusy(true); setError(""); setValue(null);
    try {
      const reply = await api.github("api_read", request);
      if (ticket !== epoch.current) return;
      if (!reply.ok) throw new Error(githubFailure(reply));
      setValue(reply.value); return reply.value;
    } catch (e) { if (ticket === epoch.current) setError(errorMessage(e)); }
    finally { if (ticket === epoch.current) setBusy(false); }
  }
  return { value, busy, error, read };
}
export function GitHubReadState({ busy, error }: { busy: boolean; error: string }) {
  return <>{busy && <p role="status">Loading GitHub data…</p>}{error && <p className="inline-error" role="alert">{error}</p>}</>;
}
export function GitHubRefSelector({ repo, select, label = "Branch or tag", allowTags = true }: { repo: string; select: (name: string, sha: string) => void; label?: string; allowTags?: boolean }) {
  const [kind, setKind] = useState("branches"), [page, setPage] = useState(1);
  const state = useGitHubRead(`${repo}:${kind}`);
  async function load(n = 1) { const v = await state.read({ operation: `repos.${kind}`, repo, parameters: { page: n, per_page: 30 } }); if (v) setPage(n); }
  const rows = Array.isArray(state.value?.data) ? state.value.data : [];
  return <div className="github-selector"><label>{label}<select aria-label={label} disabled={state.busy} defaultValue="" key={`${repo}:${kind}:${page}:${JSON.stringify(rows)}`} onChange={e => { const row = rows[Number(e.target.value)]; if (row && /^[a-f\d]{40}$/i.test(row.commit?.sha)) select(row.name, row.commit.sha); }}><option value="" disabled>Select a loaded ref…</option>{rows.map((r: any, i: number) => <option key={r.name} value={i}>{r.name}</option>)}</select></label><select aria-label={`${label} type`} value={kind} onChange={e => { setKind(e.target.value); setPage(1); }}><option value="branches">Branches</option>{allowTags && <option value="tags">Tags</option>}</select><button type="button" disabled={state.busy} onClick={() => void load()}>Load refs</button>{state.value && <><button type="button" disabled={state.busy || page === 1} onClick={() => void load(page - 1)}>Previous refs</button><span>Page {page}</span><button type="button" disabled={state.busy || !state.value.next || page >= 100} onClick={() => void load(page + 1)}>Next refs</button></>}<GitHubReadState {...state} /></div>;
}
export function GitHubQuota({ scope }: { scope: string }) {
  const state = useGitHubRead(scope);
  const buckets = state.value?.data?.resources;
  return <section className="github-api-status" aria-label="GitHub API status"><h3>API status</h3><button className="quiet-button" disabled={state.busy} onClick={() => void state.read({ operation: "rate_limit.get" })}>Refresh API quota</button><GitHubReadState {...state} />{buckets ? <dl className="github-quota">{Object.entries(buckets).filter(([, v]) => v && typeof v === "object").map(([name, raw]) => { const b = raw as any; return <div key={name}><dt>{name}</dt><dd>{b.remaining ?? "Unknown"} / {b.limit ?? "Unknown"} remaining{typeof b.reset === "number" ? ` · resets ${new Date(b.reset * 1000).toLocaleTimeString()}` : ""}</dd></div>; })}</dl> : <p className="settings-footnote">Check available requests and reset times. Data refreshes when you ask for it.</p>}</section>;
}

export function GitHubRepositoryPicker({ scope, organization, ids, change }: { scope: string; organization?: string; ids: string; change: (value: string) => void }) {
  const state = useGitHubRead(`${scope}:${organization ?? ""}`), [page, setPage] = useState(1);
  async function load(n = 1) { const v = await state.read({ operation: "repos.list", parameters: { page: n, per_page: 30, sort: "full_name" } }); if (v) setPage(n); }
  const selected = ids.split(",").map(s => s.trim()).filter(Boolean);
  const rows = (Array.isArray(state.value?.data) ? state.value.data : []).filter((r: any) => Number.isSafeInteger(r.id) && r.id > 0 && typeof r.full_name === "string" && (!organization || r.owner?.login?.toLowerCase() === organization.toLowerCase()));
  return <div><button type="button" disabled={state.busy} onClick={() => void load()}>Choose repositories by name</button><GitHubReadState {...state} />{state.value && <>{rows.map((r: any) => <label className="checkbox-label" key={r.id}><input type="checkbox" checked={selected.includes(String(r.id))} onChange={e => change((e.target.checked ? [...new Set([...selected, String(r.id)])] : selected.filter(id => id !== String(r.id))).join(", "))} />{r.full_name} · {r.id}</label>)}{!rows.length && <p>No matching repositories on this account page.</p>}<button type="button" disabled={state.busy || page === 1} onClick={() => void load(page - 1)}>Previous repository choices</button><span> Page {page} </span><button type="button" disabled={state.busy || !state.value.next || page >= 100} onClick={() => void load(page + 1)}>Next repository choices</button></>}<p className="settings-footnote">{selected.length} selected. Choices come from your paged repository access. An empty selection grants no repositories; review it deliberately.</p></div>;
}
