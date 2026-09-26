import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import { GitHubItemActions } from "./GitHubItemActions";
const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
afterEach(() => { cleanup(); vi.useRealTimers(); vi.clearAllMocks(); });
const item = { number: 7, title: "Owned title", body: "Old description", state: "open" };
function preview(request: unknown, extra = {}) { return { ok: true, value: { confirmation: "native-item", account: { host: "github.com", login: "owned" }, request, risk: "write", expires_in_seconds: 299, ...extra } }; }
function mock() { invoke.mockImplementation(async (_, { action, args }) => action === "api_preview" ? preview(args) : action === "api_confirm" ? { ok: true, value: { outcome: "applied", data: { number: 7 } } } : {}); }
const calls = (name: string) => invoke.mock.calls.filter(([, args]) => args.action === name);
function choose(value: string) { fireEvent.change(screen.getByLabelText("GitHub detail action"), { target: { value } }); }
async function review() { fireEvent.click(screen.getByText("Review item change")); return screen.findByText("Confirm item change"); }

test("issue text edits send only selected fields, require native confirmation and cannot submit via Enter", async () => {
  mock(); render(<GitHubItemActions repo="owned/repo" scope="one" item={item} />);
  expect(invoke).not.toHaveBeenCalled();
  fireEvent.change(screen.getByLabelText("Updated title"), { target: { value: "Literal <script>title</script>" } });
  fireEvent.change(screen.getByLabelText("Updated description"), { target: { value: "" } });
  const confirm = await review();
  expect(calls("api_preview")[0][1].args).toEqual({ operation: "issues.update", repo: "owned/repo", parameters: { number: 7 }, body: { title: "Literal <script>title</script>", body: "" } });
  expect(screen.getByLabelText("Exact native item change").querySelector("script")).toBeNull();
  expect(confirm.getAttribute("type")).toBe("button"); expect(calls("api_confirm")).toHaveLength(0);
  fireEvent.click(confirm); fireEvent.click(confirm);
  await screen.findByText(/GitHub accepted the request/);
  expect(calls("api_confirm")).toHaveLength(1); expect(calls("api_confirm")[0][1].args).toEqual({ confirmation: "native-item" });
});

test("PR actions use precise update, issue-metadata and reviewer schemas including explicit clearing", async () => {
  mock(); render(<GitHubItemActions repo="owned/repo" scope="one" item={{ ...item, head: { sha: "a".repeat(40) } }} />);
  const cases = [
    { action: "text", operation: "pulls.update", body: { title: item.title, body: item.body }, fill: () => {} },
    { action: "state", operation: "pulls.update", body: { state: "open" }, fill: () => fireEvent.change(screen.getByLabelText("New item state"), { target: { value: "open" } }) },
    { action: "labels", operation: "issues.update", body: { labels: ["help wanted", "a,b"] }, fill: () => fireEvent.change(screen.getByLabelText("Replacement labels"), { target: { value: "help wanted\na,b" } }) },
    { action: "assignees", operation: "issues.update", body: { assignees: [] }, fill: () => {} },
    { action: "milestone", operation: "issues.update", body: { milestone: 4 }, fill: () => fireEvent.change(screen.getByLabelText("Milestone number"), { target: { value: "4" } }) },
    { action: "milestone", operation: "issues.update", body: { milestone: null }, fill: () => fireEvent.click(screen.getByLabelText("Remove the milestone")) },
    { action: "reviewers", operation: "pulls.reviewers", body: { reviewers: ["alice"], team_reviewers: ["core"] }, fill: () => { fireEvent.change(screen.getByLabelText("Reviewer logins"), { target: { value: "alice" } }); fireEvent.change(screen.getByLabelText("Reviewer team slugs"), { target: { value: "core" } }); } },
  ];
  for (const c of cases) { choose(c.action); c.fill(); await review(); expect(calls("api_preview").at(-1)![1].args).toEqual({ operation: c.operation, repo: "owned/repo", parameters: { number: 7 }, body: c.body }); fireEvent.click(screen.getByText("Back to item form")); }
  expect(calls("api_confirm")).toHaveLength(0); expect(calls("api_dismiss")).toHaveLength(cases.length);
});

test("invalid metadata stays local and merged PRs offer no reopen", async () => {
  mock(); const ui = render(<GitHubItemActions repo="owned/repo" scope="one" item={item} />);
  choose("milestone"); fireEvent.change(screen.getByLabelText("Milestone number"), { target: { value: "-2" } }); fireEvent.click(screen.getByText("Review item change")); await screen.findByRole("alert");
  choose("labels"); fireEvent.change(screen.getByLabelText("Replacement labels"), { target: { value: "same\nsame" } }); fireEvent.click(screen.getByText("Review item change")); await screen.findByText(/distinct names/);
  expect(calls("api_preview")).toHaveLength(0);
  ui.rerender(<GitHubItemActions repo="owned/repo" scope="one" item={{ ...item, head: {}, merged: true }} />);
  expect(screen.queryByRole("option", { name: "Close or reopen" })).toBeNull();
});

test("late previews and results cannot cross item or scope changes; pending tickets are dismissed", async () => {
  let resolve!: (v: unknown) => void;
  invoke.mockImplementation(async (_, { action }) => action === "api_preview" ? new Promise(r => { resolve = r; }) : {});
  const ui = render(<GitHubItemActions repo="owned/repo" scope="one" item={item} />);
  fireEvent.click(screen.getByText("Review item change"));
  ui.rerender(<GitHubItemActions repo="owned/other" scope="two" item={{ ...item, number: 9 }} />);
  await act(async () => resolve(preview({ private: "old account" })));
  expect(screen.queryByText("Confirm item change")).toBeNull(); expect(screen.queryByText(/old account/)).toBeNull();
  expect(calls("api_dismiss")[0][1].args).toEqual({ confirmation: "native-item" });
  mock(); await review(); ui.unmount(); expect(calls("api_dismiss")).toHaveLength(2);
});

test("unknown confirmation outcomes consume the ticket and never retry", async () => {
  mock(); const ui = render(<GitHubItemActions repo="owned/repo" scope="one" item={item} />);
  const confirm = await review(); invoke.mockImplementation(async () => ({ ok: false, error: { message: "Response interrupted.", outcome: "unknown" } }));
  fireEvent.click(confirm); await screen.findByText(/change may have applied.*Inspect GitHub/);
  expect(screen.queryByText("Confirm item change")).toBeNull(); expect(calls("api_confirm")).toHaveLength(1);
  ui.unmount(); expect(calls("api_dismiss")).toHaveLength(0);
});

test("expiry and native consequential classification both gate confirmation", async () => {
  invoke.mockImplementation(async (_, { args }) => preview(args, { risk: "consequential", expires_in_seconds: 1 }));
  render(<GitHubItemActions repo="owned/repo" scope="one" item={item} />);
  const confirm = await review(); expect(confirm.hasAttribute("disabled")).toBe(true);
  fireEvent.click(screen.getByLabelText(/consequential effects/)); expect(confirm.hasAttribute("disabled")).toBe(false);
  await waitFor(() => expect(confirm.hasAttribute("disabled")).toBe(true), { timeout: 2000 });
  fireEvent.click(confirm); expect(calls("api_confirm")).toHaveLength(0);
});

test("malformed previews never authorize changes and dismiss issued tokens", async () => {
  invoke.mockImplementation(async (_, { args }) => preview(args, { account: { host: "other.host", login: "owned" } }));
  render(<GitHubItemActions repo="owned/repo" scope="one" item={item} />);
  fireEvent.click(screen.getByText("Review item change")); await screen.findByText(/Invalid native GitHub preview/);
  expect(screen.queryByText("Confirm item change")).toBeNull(); expect(calls("api_confirm")).toHaveLength(0); expect(calls("api_dismiss")).toHaveLength(1);
});
