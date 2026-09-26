import { afterEach, expect, test, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { GitHubSettings } from "./GitHubSettings";
const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
afterEach(() => { cleanup(); vi.clearAllMocks(); });
test("GitHub does not authenticate merely by opening settings", async () => {
  invoke.mockResolvedValue({ state: "disconnected", available: true });
  render(<GitHubSettings attach={vi.fn()} />);
  await waitFor(() => expect(screen.getByRole("button", { name: "Connect GitHub" }).hasAttribute("disabled")).toBe(false));
  expect(invoke.mock.calls.every(([, args]) => args.action === "status")).toBe(true);
  fireEvent.click(screen.getByRole("button", { name: "Connect GitHub" }));
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("github_command", { action: "connect", args: {} }));
});
test("an issue is previewed and requires an explicit publish click", async () => {
  invoke.mockImplementation(async (_, { action }) => action === "status" ? { state: "connected", login: "fixture" } : action === "repos" ? { items: [{ name: "fixture/project", private: false, branch: "main", url: "https://github.com/fixture/project" }] } : action === "api_preview" ? { ok: true, value: { confirmation: "native-preview", account: { login: "fixture", host: "github.com" }, request: { method: "POST", endpoint: "repos/fixture/project/issues" } } } : action === "api_confirm" ? { ok: true, value: { data: { number: 123 } } } : []);
  render(<GitHubSettings attach={vi.fn()} />);
  fireEvent.click(await screen.findByRole("button", { name: /fixture\/project/ }));
  fireEvent.click(await screen.findByRole("button", { name: "Issues" }));
  await waitFor(() => expect(screen.getByRole("button", { name: "New issue" }).hasAttribute("disabled")).toBe(false));
  fireEvent.click(screen.getByRole("button", { name: "New issue" }));
  fireEvent.change(screen.getByRole("textbox", { name: "Title" }), { target: { value: "Fix history scrolling" } });
  fireEvent.change(screen.getByRole("textbox", { name: "Message" }), { target: { value: "Keep the first message accessible." } });
  fireEvent.click(screen.getByRole("button", { name: "Review" }));
  expect(invoke.mock.calls.some(([, args]) => args.action === "create_issue")).toBe(false);
  const publish = await screen.findByRole("button", { name: "Publish to GitHub" });
  expect(screen.getByText("Keep the first message accessible.")).toBeTruthy();
  expect(invoke.mock.calls.filter(([, args]) => args.action === "api_confirm")).toHaveLength(0);
  fireEvent.click(publish);
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("github_command", { action: "api_confirm", args: { confirmation: "native-preview" } }));
  expect(invoke.mock.calls.some(([, args]) => args.action === "create_issue")).toBe(false);
});

test("clone uses shared native folder custody, never the legacy gh clone action", async () => {
  invoke.mockImplementation(async (_, { action }) => action === "status" ? { state: "connected", login: "fixture" } : action === "repos" ? { items: [{ name: "fixture/project", private: false, branch: "main", url: "https://github.com/fixture/project" }] } : []);
  const run = vi.fn().mockResolvedValue({ ticket: "native-clone", preview: { action: "clone", repository: "fixture/project", folder: "/owned" } }), effect = vi.fn();
  render(<GitHubSettings workspace="/owned" attach={vi.fn()} gitRun={run} gitEffect={effect} />);
  fireEvent.click(await screen.findByRole("button", { name: /fixture\/project/ }));
  await waitFor(() => expect(screen.getByRole("button", { name: "Clone repository" }).hasAttribute("disabled")).toBe(false));
  fireEvent.click(screen.getByRole("button", { name: "Clone repository" }));
  expect((screen.getByLabelText("Local Git operation") as HTMLSelectElement).value).toBe("clone");
  fireEvent.change(screen.getByLabelText("Clone destination"), { target: { value: "project" } });
  fireEvent.click(screen.getByText("Review local Git request"));await screen.findByText("Confirm local Git request");
  expect(run).toHaveBeenCalledWith(expect.objectContaining({ action: "git_preview", args: expect.objectContaining({ action: "clone", destination: "project", folder: "/owned" }) }));
  expect(invoke.mock.calls.some(([, args]) => args.action === "clone")).toBe(false);expect(effect).not.toHaveBeenCalled();
});

test("PR checkout enters the shared exact-head workflow instead of legacy gh checkout", async () => {
  const sha = "c".repeat(40);
  invoke.mockImplementation(async (_, { action }) => action === "status" ? { state: "connected", login: "fixture" } : action === "repos" ? { items: [{ name: "fixture/project", private: false, branch: "main", url: "https://github.com/fixture/project" }] } : action === "pulls" ? [{ id: 7, number: 7, title: "Reviewed PR", state: "open" }] : action === "item" ? { item: { number: 7, title: "Reviewed PR", state: "open", head: { sha, ref: "topic", label: "fixture:topic" }, base: { ref: "main", label: "fixture:main" } }, comments: [] } : []);
  render(<GitHubSettings workspace="/owned" attach={vi.fn()} gitRun={vi.fn()} gitEffect={vi.fn()} />);
  fireEvent.click(await screen.findByRole("button", { name: /fixture\/project/ }));
  fireEvent.click(await screen.findByRole("button", { name: "Reviewed PR" }));
  const button = await screen.findByRole("button", { name: "Check out locally" });
  await waitFor(() => expect(button.hasAttribute("disabled")).toBe(false));fireEvent.click(button);
  expect((screen.getByLabelText("Local Git operation") as HTMLSelectElement).value).toBe("fetch_pr");
  expect((screen.getByLabelText("Git pull request SHA") as HTMLInputElement).value).toBe(sha);
  expect(invoke.mock.calls.some(([, args]) => args.action === "checkout")).toBe(false);
});

test("item metadata is literal and comments page explicitly through native detail reads", async () => {
  invoke.mockImplementation(async (_, { action, args }) => action === "status" ? { state: "connected", login: "fixture" } : action === "repos" ? { items: [{ name: "fixture/project", private: false, branch: "main", url: "https://github.com/fixture/project" }] } : action === "issues" ? [{ id: 7, number: 7, title: "Owned issue" }] : action === "item" ? { item: { number: 7, title: "Owned issue", state: "open", labels: [{ name: "<script>literal</script>" }], assignees: { malformed: true }, milestone: { title: "First release" } }, comments: [{ id: args.page, user: { login: "fixture" }, body: `Comment on page ${args.page}`, created_at: "2026-09-10T12:00:00Z" }], comments_page: args.page, comments_next: args.page === 1 } : []);
  render(<GitHubSettings attach={vi.fn()} />);
  fireEvent.click(await screen.findByRole("button", { name: /fixture\/project/ })); fireEvent.click(await screen.findByRole("button", { name: "Issues" })); fireEvent.click(await screen.findByRole("button", { name: "Owned issue" }));
  await screen.findByText("Comment on page 1"); expect(screen.getByText(/Labels: <script>literal/).querySelector("script")).toBeNull();
  await waitFor(() => expect(screen.getByText("Next comments").hasAttribute("disabled")).toBe(false)); fireEvent.click(screen.getByText("Next comments"));
  await screen.findByText("Comment on page 2"); expect(screen.queryByText("Comment on page 1")).toBeNull(); expect(screen.getByText("Next comments").hasAttribute("disabled")).toBe(true);
  expect(invoke).toHaveBeenCalledWith("github_command", { action: "item", args: { repo: "fixture/project", number: 7, pull: false, page: 2 } });
  expect(screen.getByLabelText("Issue and pull request actions")).toBeTruthy(); expect(invoke.mock.calls.some(([,args]) => args.action === "api_preview" || args.action === "api_confirm")).toBe(false);
  fireEvent.click(screen.getByText("Back to list"));
  await waitFor(() => expect(screen.getByLabelText("GitHub item state filter").hasAttribute("disabled")).toBe(false));
  fireEvent.change(screen.getByLabelText("GitHub item state filter"), { target: { value: "closed" } });
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("github_command", { action: "issues", args: { repo: "fixture/project", page: 1, state: "closed" } }));
});
