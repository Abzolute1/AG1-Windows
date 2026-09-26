import { useState } from "react";
import { api } from "./bridge";
import { Markdown } from "./Markdown";
import { GitHubAction } from "./GitHubAction";
import { GitHubReadState, githubFailure, useGitHubRead } from "./GitHubControls";
export function GitHubReviewHistory({ repo, number, scope }: { repo: string; number: number; scope: string }) {
  const state = useGitHubRead(`${scope}:${repo}:${number}`), [page, setPage] = useState(1);
  async function load(n = 1) { const value = await state.read({ operation: "pulls.reviews", repo, parameters: { number, page: n, per_page: 30 } }); if (value) setPage(n); }
  return <details><summary>Review history</summary><button disabled={state.busy} onClick={() => void load()}>Load review history</button><GitHubReadState {...state} />{Array.isArray(state.value?.data) && <>{state.value.data.map((review: any) => <article className="github-comment" key={review.id}><p>@{review.user?.login ?? "Unknown"} · {review.state} · {review.submitted_at ? new Date(review.submitted_at).toLocaleString() : "Pending"}</p><Markdown text={String(review.body ?? "")} /><small>Commit: {review.commit_id}</small></article>)}{!state.value.data.length && <p>No reviews on this page.</p>}<div className="github-pagination"><button disabled={state.busy || page === 1} onClick={() => void load(page - 1)}>Previous reviews</button><span>Page {page}</span><button disabled={state.busy || !state.value.next || page >= 100} onClick={() => void load(page + 1)}>Next reviews</button></div></>}</details>;
}
const methods = [["merge", "Merge commit", "allow_merge_commit"], ["squash", "Squash", "allow_squash_merge"], ["rebase", "Rebase", "allow_rebase_merge"]];
export function GitHubMerge({ repo, number, head, scope, changed }: { repo: string; number: number; head: string; scope: string; changed: () => void }) {
  const state = useGitHubRead(`${scope}:${repo}:${number}:${head}`), [method, setMethod] = useState("");
  const allowed = methods.filter(([, , flag]) => state.value?.data?.[flag] === true);
  return <section><h3>Merge pull request #{number}</h3><p>Reviewed head: {head}</p><button disabled={state.busy} onClick={() => { setMethod(""); void state.read({ operation: "repos.get", repo }); }}>Load merge methods</button><GitHubReadState {...state} />{state.value && <GitHubAction title="Merge pull request" scope={`${scope}:${repo}:${number}:${head}`} onApplied={changed} request={async () => {
    const choice = allowed.find(([id]) => id === method); if (!choice) throw new Error("Select a method enabled by the repository.");
    const current = await api.github("api_read", { operation: "repos.get", repo });
    if (!current.ok) throw new Error(githubFailure(current));
    if (current.value?.data?.[choice[2]] !== true) throw new Error("Repository merge settings changed. Reload methods and choose again.");
    return { operation: "pulls.merge", repo, parameters: { number }, body: { sha: head, merge_method: method } };
  }}><label>Merge method<select required value={method} onChange={e => setMethod(e.target.value)}><option value="">Choose a method…</option>{allowed.map(([id, label]) => <option key={id} value={id}>{label}</option>)}</select></label>{!allowed.length && <p>No enabled merge methods were reported. Refresh repository settings or inspect GitHub.</p>}<p>GitHub verifies the exact reviewed head and repository rules. Checks alone do not authorize a merge.</p></GitHubAction>}</section>;
}
