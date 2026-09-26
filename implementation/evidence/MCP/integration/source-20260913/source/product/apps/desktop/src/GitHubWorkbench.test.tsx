import { afterEach, expect, test, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { GitHubWorkbench } from "./GitHubWorkbench";
const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
afterEach(() => { cleanup(); vi.clearAllMocks(); });
const catalog = { operations: [
  { id: "repos.list", area: "repositories", method: "GET", path: "user/repos", risk: "read" },
  { id: "repos.get", area: "repositories", method: "GET", path: "repos/{repo}", risk: "read" },
  { id: "actions.dispatch", area: "actions", method: "POST", path: "repos/{repo}/actions/workflows/{workflow_id}/dispatches", risk: "consequential" },
  { id: "api.rest", area: "advanced", method: "CUSTOM", path: "", risk: "consequential" },
  { id: "api.graphql", area: "advanced", method: "POST", path: "graphql", risk: "consequential" },
  { id: "discussions.list", area: "discussions_projects", method: "POST", path: "graphql", risk: "read" },
] };
const preview = (risk = "read") => ({ ok: true, value: { confirmation: "native-one-shot", account: { login: "fixture", host: "github.com" }, risk, warning: "Review fixture", request: { method: "GET", endpoint: "user/repos", body: null } } });
test("opening the workbench discovers locally without reading the connected account", async () => {
  invoke.mockResolvedValue(catalog); render(<GitHubWorkbench scope="first" />);
  await screen.findByRole("option", { name: "repos.list · read" });
  expect(invoke.mock.calls.map(([, args]) => args.action)).toEqual(["capabilities"]);
});
test("consequential requests require native preview, deliberate acknowledgement, and a one-shot click", async () => {
  invoke.mockImplementation(async (_, { action }) => action === "capabilities" ? catalog : action === "api_preview" ? preview("consequential") : { ok: false, error: { message: "Connection interrupted.", outcome: "unknown" } });
  render(<GitHubWorkbench scope="first" repo="owned/fixture" />);
  await screen.findByRole("option", { name: "actions.dispatch · consequential" });
  fireEvent.change(screen.getByLabelText("GitHub operation"), { target: { value: "actions.dispatch" } });
  fireEvent.change(screen.getByLabelText("GitHub parameters"), { target: { value: '{"workflow_id":"owned.yml"}' } });
  fireEvent.change(screen.getByLabelText("GitHub request body"), { target: { value: '{"ref":"main"}' } });
  fireEvent.click(screen.getByRole("button", { name: "Preview native request" }));
  const confirm = await screen.findByRole("button", { name: "Confirm GitHub change" });
  expect(confirm.hasAttribute("disabled")).toBe(true);
  fireEvent.keyDown(confirm, { key: "Enter" });
  expect(invoke.mock.calls.some(([, args]) => args.action === "api_confirm")).toBe(false);
  fireEvent.click(screen.getByRole("checkbox")); fireEvent.click(confirm); fireEvent.click(confirm);
  await screen.findByText(/change may have applied/);
  expect(invoke.mock.calls.filter(([, args]) => args.action === "api_confirm")).toHaveLength(1);
  expect(invoke).toHaveBeenCalledWith("github_command", { action: "api_confirm", args: { confirmation: "native-one-shot" } });
  expect(screen.queryByRole("button", { name: "Confirm GitHub change" })).toBeNull();
});
test("late previews after a conversation switch are discarded and their native tickets dismissed", async () => {
  let resolve!: (value: unknown) => void;
  invoke.mockImplementation(async (_, { action }) => action === "capabilities" ? catalog : action === "api_preview" ? new Promise(r => resolve = r) : { ok: true });
  const view = render(<GitHubWorkbench scope="first" />);
  await screen.findByRole("option", { name: "repos.list · read" }); fireEvent.click(screen.getByRole("button", { name: "Preview native request" }));
  await waitFor(() => expect(resolve).toBeDefined()); view.rerender(<GitHubWorkbench scope="second" />);
  resolve(preview());
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("github_command", { action: "api_dismiss", args: { confirmation: "native-one-shot" } }));
  expect(screen.queryByRole("button", { name: "Run reviewed read" })).toBeNull();
});
test("advanced GET omits a body and response source stays literal and paged", async () => {
  invoke.mockImplementation(async (_, { action }) => action === "capabilities" ? catalog : action === "api_preview" ? preview() : { ok: true, value: { status: 200, outcome: "read", data: { body: "<script>literal</script>" + "x".repeat(50000) } } });
  const { container } = render(<GitHubWorkbench scope="first" />);
  await screen.findByRole("option", { name: "api.rest · consequential" }); fireEvent.change(screen.getByLabelText("GitHub operation"), { target: { value: "api.rest" } });
  fireEvent.click(screen.getByRole("button", { name: "Preview native request" }));
  fireEvent.click(await screen.findByRole("button", { name: "Run reviewed read" }));
  const source = await screen.findByLabelText("GitHub response");
  expect(source.textContent?.length).toBeLessThanOrEqual(16000); expect(source.textContent).toContain("<script>literal</script>"); expect(container.querySelector("script")).toBeNull();
  const call = invoke.mock.calls.find(([, args]) => args.action === "api_preview")!;
  expect(call[1].args.body).toBeUndefined();
  fireEvent.click(screen.getByRole("button", { name: "Next source page" })); expect(source.textContent).not.toContain("<script>");
});
test("schema discovery is local and never confirms an account request", async () => {
  invoke.mockImplementation(async (_, { action }) => action === "capabilities" ? catalog : action === "api_search" ? { total: 1, next: null, items: [{ id: "issues/create", method: "POST", path: "repos/{owner}/{repo}/issues", summary: "Create an issue" }] } : { id: "issues/create", body: { type: "object", required: ["title"] } });
  render(<GitHubWorkbench scope="first" />);
  fireEvent.click(screen.getByText("Discover official API operations"));
  fireEvent.change(screen.getByLabelText("Search GitHub API schemas"), { target: { value: "issues create" } });
  fireEvent.click(screen.getByRole("button", { name: "Search API schemas" }));
  fireEvent.click(await screen.findByRole("button", { name: /Create an issue/ }));
  expect((await screen.findByLabelText("GitHub operation schema")).textContent).toContain('"title"');
  expect(invoke.mock.calls.every(([, args]) => ["capabilities", "api_search", "api_schema"].includes(args.action))).toBe(true);
});
test("GraphQL target labels distinguish selected discussions from account-wide queries", async () => {
  invoke.mockImplementation(async (_, { action }) => action === "capabilities" ? catalog : preview());
  render(<GitHubWorkbench scope="first" repo="owned/fixture" />);
  await screen.findByRole("option", { name: "api.graphql · consequential" });
  fireEvent.change(screen.getByLabelText("GitHub operation"), { target: { value: "discussions.list" } });
  expect(JSON.parse((screen.getByLabelText("GitHub parameters") as HTMLTextAreaElement).value)).toEqual({ owner: "owned", name: "fixture" });
  fireEvent.click(screen.getByRole("button", { name: "Preview native request" }));
  fireEvent.click(await screen.findByRole("button", { name: "Back to request" }));
  expect(invoke.mock.calls.find(([, args]) => args.action === "api_preview")![1].args.repo).toBe("owned/fixture");
  fireEvent.change(screen.getByLabelText("GitHub operation"), { target: { value: "api.graphql" } });
  expect(screen.getByText(/selected repository does not constrain/)).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Preview native request" }));
  await screen.findByRole("button", { name: "Back to request" });
  const requests = invoke.mock.calls.filter(([, args]) => args.action === "api_preview");
  expect(requests[1][1].args.repo).toBeUndefined();
  expect(requests[1][1].args.body.query).toContain("viewer");
});
test("late schema search and inspection cannot replace the current user selection", async () => {
  let oldSearch!: (value: unknown) => void, oldSchema!: (value: unknown) => void;
  invoke.mockImplementation(async (_, { action, args }) => {
    if (action === "capabilities") return catalog;
    if (action === "api_schema") return new Promise(resolve => { oldSchema = resolve; });
    if (args.query === "old") return new Promise(resolve => { oldSearch = resolve; });
    return { items: [{ id: "new", summary: "New schema", method: "GET", path: "user/repos" }], total: 1, next: null };
  });
  render(<GitHubWorkbench scope="first" />);
  await screen.findByRole("option", { name: "repos.list · read" });
  fireEvent.click(screen.getByText("Discover official API operations"));
  fireEvent.change(screen.getByLabelText("Search GitHub API schemas"), { target: { value: "old" } });
  fireEvent.click(screen.getByRole("button", { name: "Search API schemas" }));
  fireEvent.change(screen.getByLabelText("Search GitHub API schemas"), { target: { value: "new" } });
  fireEvent.click(screen.getByRole("button", { name: "Search API schemas" }));
  await screen.findByRole("button", { name: /New schema/ });
  await act(async () => { oldSearch({ items: [{ id: "old", summary: "Old schema" }], total: 1, next: null }); });
  expect(screen.queryByText("Old schema")).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "Inspect operation schema" }));
  fireEvent.change(screen.getByLabelText("GitHub operation"), { target: { value: "api.graphql" } });
  await act(async () => { oldSchema({ id: "old", summary: "Old operation schema" }); });
  expect(screen.queryByLabelText("GitHub operation schema")).toBeNull();
});
