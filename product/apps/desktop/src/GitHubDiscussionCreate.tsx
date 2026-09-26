import { useState } from "react";
import { api } from "./bridge";
import { GitHubAction } from "./GitHubAction";
import { GitHubReadState, githubFailure, useGitHubRead } from "./GitHubControls";
export function GitHubDiscussionCreate({ repo }: { repo: string }) {
  const state = useGitHubRead(repo), [repositoryId, setRepositoryId] = useState(""), [category, setCategory] = useState(""), [title, setTitle] = useState(""), [body, setBody] = useState("");
  const [owner, name] = repo.split("/");
  const categories = state.value?.data?.data?.repository?.discussionCategories;
  async function load(after?: string) {
    setCategory(""); setRepositoryId("");
    const value = await state.read({ operation: "discussions.categories", repo, parameters: { owner, name, ...(after ? { after } : {}) } });
    if (value?.data?.data?.repository?.id) setRepositoryId(value.data.data.repository.id);
  }
  return <details><summary>Create a discussion</summary><button disabled={state.busy} onClick={() => void load()}>Load discussion categories</button><GitHubReadState {...state} />{categories && <><GitHubAction title="Create discussion" scope={`${repo}:${repositoryId}:${category}`} request={async () => {
    if (!repositoryId || !categories.nodes.some((c: any) => c.id === category)) throw new Error("Select a category from this repository.");
    const current = await api.github("api_read", { operation: "repos.get", repo });
    if (!current.ok) throw new Error(githubFailure(current));
    if (current.value?.data?.node_id !== repositoryId) throw new Error("Repository identity changed. Reload discussion categories.");
    return { operation: "discussions.create", parameters: { input: { repositoryId, categoryId: category, title, body } } };
  }}><p>Repository: {repo}</p><label>Discussion category<select required value={category} onChange={e => setCategory(e.target.value)}><option value="">Choose a category…</option>{categories.nodes.map((c: any) => <option key={c.id} value={c.id}>{c.name}</option>)}</select></label><label>Discussion title<input required maxLength={256} value={title} onChange={e => setTitle(e.target.value)} /></label><label>Discussion body<textarea required rows={5} maxLength={30000} value={body} onChange={e => setBody(e.target.value)} /></label></GitHubAction>{categories.pageInfo?.hasNextPage && <button disabled={state.busy} onClick={() => void load(categories.pageInfo.endCursor)}>Next categories</button>}</>}</details>;
}
