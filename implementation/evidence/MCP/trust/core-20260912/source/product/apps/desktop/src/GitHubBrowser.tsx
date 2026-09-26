import { GitHubWorkflow } from "./GitHubWorkflow";
import { GitHubRelease } from "./GitHubRelease";
import { GitHubInbox } from "./GitHubInbox";
import { GitHubUpload } from "./GitHubUpload";
import type { GitRun } from "./GitHubLocal";
import { GitHubDownload } from "./GitHubDownload";
import { useEffect, useRef, useState } from "react";
import { api, errorMessage } from "./bridge";
import { Source } from "./GitHubWorkbench";
import { GitHubGraphDetails, type GraphTarget } from "./GitHubGraphDetails";
const views = [ ["repos.contents", "Code"], ["repos.commits", "History"], ["repos.branches", "Branches"], ["actions.workflows", "Workflows"], ["actions.runs", "Runs"], ["releases.list", "Releases"], ["repos.tags", "Tags"], ["discussions.list", "Discussions"], ["projects.list", "Organization projects"], ["projects.user_list", "Personal projects"], ["notifications.list", "Notifications"] ];
function rows(value: any): any[] {
  const data = value?.data;
  if (Array.isArray(data)) return data;
  for (const key of ["items", "workflows", "workflow_runs", "jobs", "artifacts"]) if (Array.isArray(data?.[key])) return data[key];
  const graph = data?.data;
  return graph?.repository?.discussions?.nodes ?? graph?.organization?.projectsV2?.nodes ?? graph?.user?.projectsV2?.nodes ?? graph?.node?.items?.nodes ?? [];
}
function title(item: any) { return String(item?.title ?? item?.name ?? item?.display_title ?? item?.subject?.title ?? item?.commit?.message ?? item?.content?.title ?? item?.sha ?? item?.id ?? "Item").slice(0, 300); }
function nextCursor(value: any) { const graph = value?.data?.data; const info = graph?.repository?.discussions?.pageInfo ?? graph?.organization?.projectsV2?.pageInfo ?? graph?.user?.projectsV2?.pageInfo ?? graph?.node?.items?.pageInfo; return info?.hasNextPage && typeof info.endCursor === "string" ? info.endCursor : null; }
export function GitHubBrowser({ repo, workspace, run }: { repo: string; workspace?: string; run?: GitRun }) {
  const [expanded, setExpanded] = useState<string | null>(null);
  const [graphTarget, setGraphTarget] = useState<GraphTarget | null>(null);
  const [operation, setOperation] = useState("repos.contents"), [path, setPath] = useState(""), [value, setValue] = useState<any>(null), [item, setItem] = useState<any>(null);
  const [page, setPage] = useState(1), [cursor, setCursor] = useState<string | null>(null), [error, setError] = useState(""), [busy, setBusy] = useState(false);
  const [loaded, setLoaded] = useState<{ operation: string; parameters: Record<string, unknown> } | null>(null);
  const epoch = useRef(0);
  useEffect(() => { ++epoch.current; setValue(null); setItem(null); setExpanded(null); setGraphTarget(null); setError(""); setPage(1); setCursor(null); setPath(""); setBusy(false); return () => { ++epoch.current; }; }, [repo, operation]);
  async function read(selected = operation, params?: Record<string, unknown>, nextPage = 1, after: string | null = null) {
    const ticket = ++epoch.current; setBusy(true); setError(""); setItem(null); setExpanded(null);
    try {
      const [owner, name] = repo.split("/");
      let parameters: Record<string, unknown> = { page: nextPage, per_page: 30 };
      if (selected === "repos.contents") parameters = { path };
      if (selected === "discussions.list") parameters = { owner, name, ...(after ? { after } : {}) };
      if (["projects.list", "projects.user_list"].includes(selected)) parameters = { login: owner, ...(after ? { after } : {}) };
      if (params) parameters = params;
      const result = await api.github("api_read", { operation: selected, ...(selected === "notifications.list" || selected.startsWith("projects.") ? {} : { repo }), parameters });
      if (ticket !== epoch.current) return;
      if (!result.ok) throw new Error(result.error?.message ?? "GitHub read failed.");
      if (!Array.isArray(rows(result.value))) throw new Error("Invalid GitHub item list.");
      setValue(result.value); setPage(nextPage); setCursor(nextCursor(result.value)); setLoaded({ operation: selected, parameters });
    } catch (e) { if (ticket === epoch.current) setError(errorMessage(e)); }
    finally { if (ticket === epoch.current) setBusy(false); }
  }
  const items = rows(value).filter(row => row && typeof row === "object").slice(0, 100);
  function turnPage(nextPage: number) { if (!loaded) return; const graph = loaded.operation.startsWith("projects.") || loaded.operation.startsWith("discussions."); void read(loaded.operation, { ...loaded.parameters, ...(graph ? { after: cursor } : { page: nextPage }) }, nextPage, cursor); }
  return <section className="settings-card github-browser" aria-label="GitHub repository browser">
    <h3>Repository and account views</h3><label>View<select aria-label="GitHub repository view" disabled={busy} value={operation} onChange={e => setOperation(e.target.value)}>{views.map(([id, label]) => <option value={id} key={id}>{label}</option>)}</select></label>
    {operation === "repos.contents" && <label>Repository path<input aria-label="GitHub repository path" value={path} maxLength={2048} onChange={e => setPath(e.target.value)} placeholder="Folder or file path (empty for root)" /></label>}
    {operation.startsWith("projects.") && <p>Projects owned by {repo.split("/")[0]}; this is an account view, not a repository-scoped project list.</p>}
    {operation === "notifications.list" && <GitHubInbox repo={repo} scope={repo} />}
    {operation === "releases.list" && <details><summary>Create a release</summary><GitHubRelease repo={repo} scope={repo} /></details>}
    {operation !== "notifications.list" && <button className="outline-button" disabled={busy} onClick={() => void read()}>{value ? "Refresh view" : "Load view"}</button>}
    {busy && <p role="status">Loading GitHub view…</p>}{error && <p className="inline-error" role="alert">{error}</p>}
    {value && <><p>Showing {loaded?.operation}</p><div className="github-items">{items.map((row, index) => <button key={`${row.id ?? row.sha ?? row.path ?? index}`} onClick={() => { setExpanded(null); setItem(row); }}><span><strong>{title(row)}</strong><small>{String(row.state ?? row.status ?? row.type ?? row.reason ?? "").slice(0, 100)}{row.conclusion ? ` · ${String(row.conclusion).slice(0, 100)}` : ""}</small></span></button>)}</div>
    {!items.length && <p>{Array.isArray(value.data) ? "No items on this page." : "Open response details to inspect this result."}</p>}
    {!loaded?.operation.includes("contents") && <div className="github-pagination"><button disabled={busy || page === 1 || loaded?.operation.startsWith("projects.") || loaded?.operation.startsWith("discussions.")} onClick={() => turnPage(page - 1)}>Previous page</button><span>Page {page}</span><button disabled={busy || !(value.next || cursor)} onClick={() => turnPage(page + 1)}>Next page</button></div>}
    <details><summary>Complete response details</summary><Source value={value.data} label="GitHub browser response" /></details></>}
    {item && <div className="github-browser-detail"><h4>{title(item)}</h4>
      {["actions.runs", "actions.workflow_runs"].includes(loaded?.operation ?? "") && <div className="github-actions"><button disabled={busy} onClick={() => void read("actions.jobs", { run_id: item.id, page: 1, per_page: 30 })}>Inspect jobs</button><button disabled={busy} onClick={() => void read("actions.artifacts", { run_id: item.id, page: 1, per_page: 30 })}>Inspect run artifacts</button></div>}
      {loaded?.operation === "actions.workflows" && <button disabled={busy} onClick={() => void read("actions.workflow_runs", { workflow_id: item.id, page: 1, per_page: 30 })}>Inspect workflow runs</button>}
      {["actions.workflows", "actions.runs", "actions.workflow_runs", "actions.jobs"].includes(loaded?.operation ?? "") && Number.isSafeInteger(item.id) && item.id > 0 && <details onToggle={e => setExpanded(e.currentTarget.open ? "workflow" : null)}><summary>Workflow details and actions</summary>{expanded === "workflow" && <GitHubWorkflow key={`${repo}:${loaded?.operation}:${item.id}`} repo={repo} kind={loaded?.operation === "actions.workflows" ? "workflow" : loaded?.operation === "actions.jobs" ? "job" : "run"} id={item.id} scope={repo} />}</details>}
      {loaded?.operation === "releases.list" && Number.isSafeInteger(item.id) && item.id > 0 && <details onToggle={e => setExpanded(e.currentTarget.open ? "release" : null)}><summary>Release details and editing</summary>{expanded === "release" && <GitHubRelease key={`${repo}:${item.id}`} repo={repo} id={item.id} scope={repo} />}</details>}
      {loaded?.operation === "releases.list" && <button disabled={busy} onClick={() => void read("releases.assets", { release_id: item.id, page: 1, per_page: 30 })}>Inspect release assets</button>}
      {["projects.list", "projects.user_list"].includes(loaded?.operation ?? "") && typeof item.id === "string" && <button disabled={busy} onClick={() => setGraphTarget({ kind: "project", id: item.id })}>Inspect project items and fields</button>}
      {loaded?.operation === "discussions.list" && Number.isSafeInteger(item.number) && item.number > 0 && <button disabled={busy} onClick={() => setGraphTarget({ kind: "discussion", repo, number: item.number })}>Inspect discussion and replies</button>}
      {loaded?.operation === "releases.list" && Number.isSafeInteger(item.id) && item.id > 0 && <details><summary>Upload an asset to this release</summary><GitHubUpload key={`${repo}:${item.id}:${workspace}`} repo={repo} release={item.id} workspace={workspace} run={run} /></details>}
      {Number.isSafeInteger(item.id) && item.id > 0 && ["actions.runs", "actions.workflow_runs", "actions.jobs", "actions.artifacts", "releases.assets"].includes(loaded?.operation ?? "") && <GitHubDownload key={`${repo}:${loaded?.operation}:${item.id}`} scope={repo} label={loaded?.operation === "actions.runs" ? "Download run logs" : loaded?.operation === "actions.jobs" ? "Download job log" : "Download file"} request={{ repo, operation: ({ "actions.runs": "actions.download_logs", "actions.workflow_runs": "actions.download_logs", "actions.jobs": "actions.download_job_log", "actions.artifacts": "actions.download_artifact", "releases.assets": "releases.download_asset" } as Record<string, string>)[loaded!.operation], parameters: { [({ "actions.runs": "run_id", "actions.workflow_runs": "run_id", "actions.jobs": "job_id", "actions.artifacts": "artifact_id", "releases.assets": "asset_id" } as Record<string, string>)[loaded!.operation]]: item.id } }} />}
      <Source value={item} label="Selected GitHub item" /><button onClick={() => setItem(null)}>Close item</button>
    </div>}
    {graphTarget && <GitHubGraphDetails key={`${repo}:${JSON.stringify(graphTarget)}`} target={graphTarget} scope={repo} close={() => setGraphTarget(null)} />}
    <p className="settings-footnote">Use the API workbench to prepare changes to the selected item; it always shows the native account, target and fields before confirmation.</p>
  </section>;
}
