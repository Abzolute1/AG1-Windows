import { afterEach, expect, test, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { GitHubBrowser } from "./GitHubBrowser";
const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
afterEach(() => { cleanup(); vi.clearAllMocks(); });
test("root code browse is an explicit bounded native read", async () => {
  invoke.mockResolvedValue({ ok: true, value: { data: [{ name: "README.md", path: "README.md", type: "file" }] } });
  render(<GitHubBrowser repo="owned/fixture" />); expect(invoke).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "Load view" }));
  await screen.findByText("README.md");
  expect(invoke).toHaveBeenCalledWith("github_command", { action: "api_read", args: { operation: "repos.contents", repo: "owned/fixture", parameters: { path: "" } } });
});
test("nested job paging preserves the run target and does not treat jobs as runs", async () => {
  invoke.mockImplementation(async (_, { args }) => ({ ok: true, value: args.operation === "actions.runs" ? { data: { workflow_runs: [{ id: 55, display_title: "Owned run" }] } } : { data: { jobs: [{ id: 66, name: "Owned job" }] }, next: "next" } }));
  render(<GitHubBrowser repo="owned/fixture" />);
  fireEvent.change(screen.getByLabelText("GitHub repository view"), { target: { value: "actions.runs" } }); fireEvent.click(screen.getByRole("button", { name: "Load view" }));
  fireEvent.click(await screen.findByRole("button", { name: "Owned run" })); fireEvent.click(screen.getByRole("button", { name: "Inspect jobs" }));
  fireEvent.click(await screen.findByRole("button", { name: "Owned job" })); expect(screen.queryByRole("button", { name: "Inspect jobs" })).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "Next page" }));
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("github_command", { action: "api_read", args: { operation: "actions.jobs", repo: "owned/fixture", parameters: { run_id: 55, page: 2, per_page: 30 } } }));
});
test("private replies from the previous repository are ignored", async () => {
  let resolve!: (value: unknown) => void; invoke.mockImplementation(() => new Promise(r => resolve = r));
  const view = render(<GitHubBrowser repo="owned/first" />); fireEvent.click(screen.getByRole("button", { name: "Load view" }));
  view.rerender(<GitHubBrowser repo="owned/second" />); resolve({ ok: true, value: { data: [{ name: "old private result" }] } });
  await waitFor(() => expect(screen.getByRole("button", { name: "Load view" }).hasAttribute("disabled")).toBe(false));
  expect(screen.queryByText("old private result")).toBeNull();
});
