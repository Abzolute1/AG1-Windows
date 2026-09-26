import { useState } from "react";
import { GitHubAction } from "./GitHubAction";
import { GitHubReadState, useGitHubRead } from "./GitHubControls";
export function GitHubMetadataPicker({ repo, scope, kind, selected, select, single = false, titleValue = false }: { repo: string; scope: string; kind: "labels" | "milestones"; selected: string; select: (value: string) => void; single?: boolean; titleValue?: boolean }) {
  const read = useGitHubRead(`${scope}:${repo}:${kind}`), [page, setPage] = useState(1);
  async function load(n = 1) { const value = await read.read({ operation: `issues.${kind}`, repo, parameters: { page: n, per_page: 30, ...(kind === "milestones" ? { state: "open" } : {}) } }); if (value) setPage(n); }
  const rows = Array.isArray(read.value?.data) ? read.value.data : [];
  return <div><button type="button" disabled={read.busy} onClick={() => void load()}>Load {kind}</button><GitHubReadState {...read} />{read.value && <><label>Choose {kind === "labels" ? "label to include" : "milestone"}<select value="" onChange={e => { if (kind === "labels" && !single) select([...new Set([...selected.split("\n").filter(Boolean), e.target.value])].join("\n")); else select(e.target.value); }}><option value="">Choose…</option>{rows.map((row: any) => <option key={row.id} value={kind === "labels" ? row.name : titleValue ? row.title : String(row.number)}>{row.name ?? row.title}</option>)}</select></label><button type="button" disabled={read.busy || page === 1} onClick={() => void load(page - 1)}>Previous {kind}</button><span> Page {page} </span><button type="button" disabled={read.busy || !read.value.next || page >= 100} onClick={() => void load(page + 1)}>Next {kind}</button></>}{kind === "milestones" && !titleValue && <p className="settings-footnote">Open milestones are listed. The number field also accepts a known closed milestone.</p>}</div>;
}
export function GitHubMilestoneCreate({ repo, scope }: { repo: string; scope: string }) {
  const [title, setTitle] = useState("");
  return <details><summary>Create a milestone</summary><GitHubAction title="Create milestone" scope={`${scope}:${repo}`} request={() => ({ operation: "issues.create_milestone", repo, body: { title } })}><label>Milestone title<input required maxLength={256} value={title} onChange={e => setTitle(e.target.value)} /></label></GitHubAction></details>;
}
