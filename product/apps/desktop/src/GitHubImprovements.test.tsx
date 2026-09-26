import { afterEach, expect, test, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { GitHubSettings } from "./GitHubSettings";
import { GitHubRepository } from "./GitHubRepository";
import { GitHubAdministration } from "./GitHubAdministration";
import { GitHubMerge } from "./GitHubReviewActions";
import { GitHubInlineReview, completeReviewPatch } from "./GitHubInlineReview";
import { GitHubBrowser } from "./GitHubBrowser";
import { GitHubDiscussionCreate } from "./GitHubDiscussionCreate";
import { GitHubLocal } from "./GitHubLocal";
import { GitHubRepositoryPicker, githubFailure } from "./GitHubControls";
const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
afterEach(() => { cleanup(); vi.clearAllMocks(); });
const sha = "a".repeat(40), base = "b".repeat(40), repo = "owned/project";
const repository = { id: 42, name: repo, full_name: repo, branch: "main", default_branch: "main", private: true, url: `https://github.com/${repo}`, description: "Original", allow_merge_commit: true, allow_squash_merge: false, allow_rebase_merge: true };
const ok = (data: unknown, next?: string) => ({ ok: true, value: { data, next } });
function fixture(read: (args: any) => any = () => ok([])) {
  invoke.mockImplementation(async (_, { action, args }) => {
    if (action === "status") return { state: "connected", login: "owned" };
    if (action === "repos") return { items: [repository], has_more: true };
    if (action === "api_read") return read(args);
    if (action === "api_preview") return { ok: true, value: { confirmation: "owned-preview", account: { host: "github.com", login: "owned" }, risk: "consequential", request: args, expires_in_seconds: 300 } };
    if (action === "api_confirm") return ok({ changed: true });
    if (action === "capabilities") return { operations: [{ id: "repos.get", area: "repositories", path: "repos/{repo}", method: "GET", risk: "read" }] };
    if (action === "api_schema") return {};
    return { items: [], has_more: false };
  });
}
const requestCalls = () => invoke.mock.calls.filter(([,a]) => a.action === "api_preview").map(([,a]) => a.args);
async function repositories() { fireEvent.click(await screen.findByRole("button", { name: "Browse repositories" })); await screen.findByRole("button", { name: repo }); }

test("Overview keeps Advanced opt-in and repository search uses native search with real next metadata", async () => {
  fixture(args => args.operation === "repos.search" ? ok({ items: [{ ...repository, full_name: "owned/search-result" }] }, "next") : ok([]));
  render(<GitHubSettings attach={vi.fn()} />);
  await screen.findByRole("button", { name: "Browse repositories" });
  expect(invoke.mock.calls.some(([, a]) => a.action === "capabilities")).toBe(false);
  await repositories();
  fireEvent.change(screen.getByLabelText("Repository source"), { target: { value: "search" } });
  fireEvent.change(screen.getByLabelText("Search repositories"), { target: { value: "owner:owned rust" } });
  expect(invoke.mock.calls.some(([, a]) => a.args?.operation === "repos.search")).toBe(false);
  fireEvent.click(screen.getByRole("button", { name: "Search repositories" }));
  await screen.findByRole("button", { name: "owned/search-result" });
  fireEvent.click(screen.getByRole("button", { name: "Next" }));
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("github_command", { action: "api_read", args: { operation: "repos.search", parameters: { q: "owner:owned rust", page: 2, per_page: 30 } } }));
  fireEvent.click(screen.getByRole("button", { name: "Advanced" }));
  await waitFor(() => expect(invoke.mock.calls.some(([, a]) => a.action === "capabilities")).toBe(true));
});
test("issue searches retain fixed repository/type/state scope and filters on later pages", async () => {
  fixture(() => ok({ items: [{ id: 1, number: 1, title: "Result" }] }, "next"));
  render(<GitHubSettings attach={vi.fn()} />); await repositories();
  fireEvent.click(screen.getByRole("button", { name: repo }));
  fireEvent.click(await screen.findByRole("button", { name: "Issues" }));
  await waitFor(() => expect(screen.getByRole("button", { name: "Search items" }).hasAttribute("disabled")).toBe(false));
  fireEvent.change(screen.getByLabelText("Search issues or pull requests"), { target: { value: "parser" } });
  fireEvent.change(screen.getByLabelText("Assignee filter"), { target: { value: "owned" } });
  fireEvent.click(screen.getByRole("button", { name: "Search items" })); await screen.findByRole("button", { name: "Result" });
  fireEvent.click(screen.getByRole("button", { name: "Next" }));
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("github_command", { action: "api_read", args: { operation: "issues.search", parameters: { q: `parser assignee:owned repo:${repo} is:issue is:open`, page: 2, per_page: 30 } } }));
  fireEvent.change(screen.getByLabelText("Search issues or pull requests"), { target: { value: "(repo:another/project)" } });
  fireEvent.click(screen.getByRole("button", { name: "Search items" }));
  expect((await screen.findByRole("alert")).textContent).toContain("scope");
});
test("repository configuration sends only edited fields and keeps risky state changes separate", async () => {
  fixture(() => ok(repository)); render(<GitHubRepository repo={repo} scope="owned" />);
  fireEvent.click(screen.getByRole("button", { name: "Load repository settings" }));
  fireEvent.change(await screen.findByLabelText("Description"), { target: { value: "Updated" } });
  fireEvent.click(within(screen.getByRole("region", { name: "Repository configuration" })).getByRole("button", { name: "Review change" }));
  await screen.findByRole("button", { name: "Confirm reviewed change" });
  expect(requestCalls()).toEqual([{ operation: "repos.update", repo, body: { description: "Updated" } }]);
  expect(invoke.mock.calls.some(([, a]) => a.action === "api_confirm")).toBe(false);
});
test("merge methods respect reported settings, recheck changes and retain exact head", async () => {
  fixture(() => ok(repository)); render(<GitHubMerge repo={repo} number={7} head={sha} scope="owned" changed={vi.fn()} />);
  fireEvent.click(screen.getByRole("button", { name: "Load merge methods" }));
  const choice = await screen.findByLabelText("Merge method"); expect(within(choice).queryByRole("option", { name: "Squash" })).toBeNull();
  fireEvent.change(choice, { target: { value: "rebase" } }); fireEvent.click(screen.getByRole("button", { name: "Review change" }));
  await screen.findByRole("button", { name: "Confirm reviewed change" });
  expect(requestCalls()[0]).toEqual({ operation: "pulls.merge", repo, parameters: { number: 7 }, body: { sha, merge_method: "rebase" } });
  expect(screen.getByRole("button", { name: "Confirm reviewed change" }).hasAttribute("disabled")).toBe(true);
});
test("inline review refuses incomplete patches and submits only a checked exact anchor", async () => {
  const file = { filename: "src/file.ts", additions: 1, deletions: 1, patch: "@@ -1 +1 @@\n-old\n+new" };
  expect(completeReviewPatch(file)).toBe(true);
  expect(completeReviewPatch({ ...file, patch: "@@ -1 +1,2 @@\n-old\n+new" })).toBe(false);
  expect(completeReviewPatch({ ...file, additions: 2 })).toBe(false);
  fixture(() => ok({ head: { sha }, base: { sha: base } }));
  render(<GitHubInlineReview repo={repo} number={7} head={sha} base={base} file={file} />);
  fireEvent.click(screen.getByText("Comment on a diff line"));
  fireEvent.change(screen.getByLabelText("Diff line"), { target: { value: "RIGHT:1" } });
  fireEvent.change(screen.getByLabelText("Inline comment"), { target: { value: "Check this line" } });
  fireEvent.click(screen.getByRole("button", { name: "Review change" }));
  await screen.findByRole("button", { name: "Confirm reviewed change" });
  expect(requestCalls()[0]).toEqual({ operation: "pulls.review", repo, parameters: { number: 7 }, body: { event: "COMMENT", commit_id: sha, comments: [{ path: "src/file.ts", side: "RIGHT", line: 1, body: "Check this line" }] } });
});
test("inline review changed identities refuse preview without losing the draft to a different commit", async () => {
  fixture(() => ok({ head: { sha: "c".repeat(40) }, base: { sha: base } }));
  render(<GitHubInlineReview repo={repo} number={7} head={sha} base={base} file={{ filename: "file", additions: 1, deletions: 0, patch: "@@ -0,0 +1 @@\n+new" }} />);
  fireEvent.click(screen.getByText("Comment on a diff line")); fireEvent.change(screen.getByLabelText("Diff line"), { target: { value: "RIGHT:1" } }); fireEvent.change(screen.getByLabelText("Inline comment"), { target: { value: "Draft" } }); fireEvent.click(screen.getByRole("button", { name: "Review change" }));
  expect((await screen.findByRole("alert")).textContent).toContain("head or base changed"); expect(requestCalls()).toHaveLength(0);
});
test("variable list is narrow read and deletion is an exact reviewed raw request", async () => {
  fixture(() => ok({ variables: [{ name: "SETTING", value: "plain" }] })); render(<GitHubAdministration repo={repo} scope="owned" kind="variables" />);
  fireEvent.click(screen.getByRole("button", { name: "Load variables" })); await screen.findByText("plain");
  expect(invoke).toHaveBeenCalledWith("github_command", { action: "api_read", args: { operation: "variables.repository_list", repo, parameters: { page: 1, per_page: 30 } } });
  fireEvent.click(screen.getByRole("button", { name: "Select" })); fireEvent.change(screen.getByLabelText("Action"), { target: { value: "delete" } }); fireEvent.click(screen.getByRole("button", { name: "Review change" }));
  await screen.findByRole("button", { name: "Confirm reviewed change" }); expect(requestCalls()[0]).toEqual({ operation: "api.rest", repo, parameters: { method: "DELETE", endpoint: `repos/${repo}/actions/variables/SETTING` } });
});
test("environment edits use typed encoded-path operations and scope changes dismiss previews", async () => {
  fixture(); render(<GitHubAdministration repo={repo} scope="owned" kind="variables" />);
  fireEvent.change(screen.getByLabelText("Variable scope"), { target: { value: "environment" } }); fireEvent.change(screen.getByLabelText("Variable environment"), { target: { value: "Production / EU" } });
  fireEvent.change(screen.getByLabelText("Variable name"), { target: { value: "SETTING" } }); fireEvent.change(screen.getByLabelText("Plaintext variable value"), { target: { value: "plain" } }); fireEvent.click(screen.getByRole("button", { name: "Review change" }));
  await screen.findByRole("button", { name: "Confirm reviewed change" }); expect(requestCalls()[0]).toEqual({ operation: "variables.environment_create", repo, parameters: { environment_name: "Production / EU" }, body: { name: "SETTING", value: "plain" } });
  fireEvent.change(screen.getByLabelText("Variable environment"), { target: { value: "Staging" } });
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("github_command", { action: "api_dismiss", args: { confirmation: "owned-preview" } })); expect(screen.queryByRole("button", { name: "Confirm reviewed change" })).toBeNull();
});
test("collaborator requests target the named user and require native review", async () => {
  fixture(); render(<GitHubAdministration repo={repo} scope="owned" kind="access" />);
  fireEvent.change(screen.getByLabelText("Collaborator login"), { target: { value: "colleague" } }); fireEvent.change(screen.getByLabelText("Repository permission"), { target: { value: "triage" } }); fireEvent.click(screen.getByRole("button", { name: "Review change" }));
  await screen.findByRole("button", { name: "Confirm reviewed change" }); expect(requestCalls()[0]).toEqual({ operation: "api.rest", repo, parameters: { method: "PUT", endpoint: `repos/${repo}/collaborators/colleague` }, body: { permission: "triage" } });
});
test("Projects owner is independent and late prior-owner replies are discarded", async () => {
  let finish!: (v: any) => void; invoke.mockImplementation(() => new Promise(resolve => finish = resolve)); render(<GitHubBrowser repo={repo} />);
  fireEvent.change(screen.getByLabelText("GitHub repository view"), { target: { value: "projects.list" } }); fireEvent.change(screen.getByLabelText("Project owner login"), { target: { value: "separate-org" } }); fireEvent.click(screen.getByRole("button", { name: "Load view" }));
  expect(invoke).toHaveBeenCalledWith("github_command", { action: "api_read", args: { operation: "projects.list", parameters: { login: "separate-org" } } });
  fireEvent.change(screen.getByLabelText("Project owner login"), { target: { value: "new-org" } }); finish(ok({ data: { organization: { projectsV2: { nodes: [{ id: "P", title: "Old private project" }] } } } }));
  await waitFor(() => expect(screen.getByRole("button", { name: "Load view" }).hasAttribute("disabled")).toBe(false)); expect(screen.queryByText("Old private project")).toBeNull();
});
test("Discussion creation uses repository and category node IDs without claiming graph repo confinement", async () => {
  fixture(args => args.operation === "repos.get" ? ok({ node_id: "R_owned" }) : ok({ data: { repository: { id: "R_owned", discussionCategories: { nodes: [{ id: "C_owned", name: "Ideas" }], pageInfo: { hasNextPage: false } } } } }));
  render(<GitHubDiscussionCreate repo={repo} />); fireEvent.click(screen.getByText("Create a discussion")); fireEvent.click(screen.getByRole("button", { name: "Load discussion categories" }));
  fireEvent.change(await screen.findByLabelText("Discussion category"), { target: { value: "C_owned" } }); fireEvent.change(screen.getByLabelText("Discussion title"), { target: { value: "Idea" } }); fireEvent.change(screen.getByLabelText("Discussion body"), { target: { value: "Discuss this" } }); fireEvent.click(screen.getByRole("button", { name: "Review change" }));
  await screen.findByRole("button", { name: "Confirm reviewed change" }); expect(requestCalls()[0]).toEqual({ operation: "discussions.create", parameters: { input: { repositoryId: "R_owned", categoryId: "C_owned", title: "Idea", body: "Discuss this" } } });
});
test("local branch selector carries exact SHA into the existing native plan", async () => {
  fixture(() => ok([{ name: "topic", commit: { sha } }])); const run = vi.fn().mockResolvedValue({ ticket: "local", preview: { repository: repo, folder: "/owned", action: "fetch" } });
  render(<GitHubLocal repo={repo} workspace="/owned" run={run} effect={vi.fn()} />); fireEvent.change(screen.getByLabelText("Local Git operation"), { target: { value: "fetch" } }); fireEvent.click(screen.getByRole("button", { name: "Load refs" })); await screen.findByRole("option", { name: "topic" });
  fireEvent.change(screen.getByLabelText("Choose Git branch and commit"), { target: { value: "0" } }); fireEvent.click(screen.getByRole("button", { name: "Review local Git request" }));
  await waitFor(() => expect(run).toHaveBeenCalledWith({ kind: "files", action: "git_preview", args: { action: "fetch", repo, folder: "/owned", branch: "topic", expected_remote_sha: sha } }));
});
test("named repository selection preserves selections from other pages", async () => {
  fixture(() => ok([{ id: 42, full_name: repo, owner: { login: "owned" } }], "next")); const change = vi.fn(); render(<GitHubRepositoryPicker scope="owned" organization="owned" ids="73" change={change} />); fireEvent.click(screen.getByRole("button", { name: "Choose repositories by name" })); fireEvent.click(await screen.findByRole("checkbox", { name: `${repo} · 42` })); expect(change).toHaveBeenCalledWith("73, 42");
  expect(githubFailure({ error: { code: "rate_limit", message: "Wait", retry_after_seconds: 10 } })).toContain("10 seconds");
});

test("leaving a repository search discards late data and does not block the next section", async () => {
  let finish!: (v: any) => void;
  fixture(args => args.operation === "repos.search" ? new Promise(resolve => { finish = resolve; }) : ok([]));
  render(<GitHubSettings attach={vi.fn()} />); await repositories();
  fireEvent.change(screen.getByLabelText("Repository source"), { target: { value: "search" } }); fireEvent.change(screen.getByLabelText("Search repositories"), { target: { value: "pending" } }); fireEvent.click(screen.getByRole("button", { name: "Search repositories" }));
  await waitFor(() => expect(finish).toBeTypeOf("function"));
  fireEvent.click(screen.getByRole("button", { name: "Overview" }));
  finish(ok({ items: [{ ...repository, full_name: "owned/stale-private" }] }));
  await waitFor(() => expect(screen.queryByText("Working with GitHub…")).toBeNull());
  fireEvent.click(screen.getByRole("button", { name: "Browse repositories" })); expect(screen.queryByRole("button", { name: "owned/stale-private" })).toBeNull();
});
test("changed repository merge settings prevent preparing an obsolete selected method", async () => {
  let reads = 0; fixture(() => ok({ ...repository, allow_rebase_merge: ++reads === 1 }));
  render(<GitHubMerge repo={repo} number={7} head={sha} scope="owned" changed={vi.fn()} />); fireEvent.click(screen.getByRole("button", { name: "Load merge methods" }));
  fireEvent.change(await screen.findByLabelText("Merge method"), { target: { value: "rebase" } }); fireEvent.click(screen.getByRole("button", { name: "Review change" }));
  expect((await screen.findByRole("alert")).textContent).toContain("merge settings changed"); expect(requestCalls()).toHaveLength(0);
});
