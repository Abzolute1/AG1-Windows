import { afterEach, expect, test, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { GitHubLocal } from "./GitHubLocal";
afterEach(cleanup);
const preview = { ticket: "native-one-shot", preview: { repository: "owned/repo", folder: "/owned", action: "stage", files: [{ path: " notes.txt ", sha256: "native-hash" }] } };
test("local Git uses native exact preview and one-shot effect custody", async () => {
  const run = vi.fn().mockResolvedValue(preview), effect = vi.fn().mockResolvedValue({ ok: false, outcome: "unknown" });
  render(<GitHubLocal repo="owned/repo" workspace="/owned" run={run} effect={effect} />);
  fireEvent.change(screen.getByLabelText("Local Git operation"), { target: { value: "stage" } });
  fireEvent.change(screen.getByLabelText("Git paths"), { target: { value: " notes.txt " } });
  fireEvent.click(screen.getByText("Review local Git request"));
  const confirm = await screen.findByText("Confirm local Git request");
  expect(run).toHaveBeenCalledWith({ kind: "files", action: "git_preview", args: { action: "stage", repo: "owned/repo", folder: "/owned", paths: [" notes.txt "] } });
  expect(effect).not.toHaveBeenCalled();
  fireEvent.keyDown(confirm, { key: "Enter" }); expect(effect).not.toHaveBeenCalled();
  fireEvent.click(confirm); fireEvent.click(confirm);
  await screen.findByRole("alert");
  expect(effect).toHaveBeenCalledTimes(1);
  expect(effect).toHaveBeenCalledWith({ kind: "files", action: "git_confirm", args: { ticket: "native-one-shot" } });
  expect(screen.queryByText("Confirm local Git request")).toBeNull();
});
test("late local previews are discarded when the repository changes", async () => {
  let resolve!: (value: any) => void;
  const run = vi.fn().mockImplementation(({ action }) => action === "git_preview" ? new Promise(r => resolve = r) : Promise.resolve({}));
  const effect = vi.fn();
  const view = render(<GitHubLocal repo="owned/repo" workspace="/owned" run={run} effect={effect} />);
  fireEvent.click(screen.getByText("Review local Git request"));
  view.rerender(<GitHubLocal repo="other/repo" workspace="/owned" run={run} effect={effect} />);
  resolve(preview);
  await waitFor(() => expect(run).toHaveBeenCalledWith({ kind: "files", action: "git_dismiss", args: { ticket: "native-one-shot" } }));
  expect(screen.queryByText("Confirm local Git request")).toBeNull(); expect(effect).not.toHaveBeenCalled();
});
test("editing an argument dismisses the old native plan", async () => {
  const run = vi.fn().mockResolvedValue(preview), effect = vi.fn();
  render(<GitHubLocal repo="owned/repo" workspace="/owned" run={run} effect={effect} />);
  fireEvent.click(screen.getByText("Review local Git request")); await screen.findByText("Confirm local Git request");
  fireEvent.change(screen.getByLabelText("Local Git operation"), { target: { value: "commit" } });
  expect(run).toHaveBeenCalledWith({ kind: "files", action: "git_dismiss", args: { ticket: "native-one-shot" } });
  expect(screen.queryByText("Confirm local Git request")).toBeNull(); expect(effect).not.toHaveBeenCalled();
});
test("push binds exact branch/SHA and requires deliberate CI acknowledgement", async () => {
  const run = vi.fn().mockResolvedValue({ ticket: "native-push", preview: { repository: "owned/repo", folder: "/owned", action: "push", account: { login: "owned", host: "github.com" }, network_warning: "May trigger CI." } });
  const effect = vi.fn().mockResolvedValue({ ok: true, result: { operation: "push", outcome: "applied" } });
  render(<GitHubLocal repo="owned/repo" workspace="/owned" run={run} effect={effect} />);
  fireEvent.change(screen.getByLabelText("Local Git operation"), { target: { value: "push" } });
  fireEvent.change(screen.getByLabelText("Git remote branch"), { target: { value: "reviewed-branch" } });
  fireEvent.change(screen.getByLabelText("Reviewed remote SHA"), { target: { value: "a".repeat(40) } });
  fireEvent.click(screen.getByText("Review local Git request"));
  const confirm = await screen.findByText("Confirm local Git request");
  expect(run).toHaveBeenCalledWith({ kind: "files", action: "git_preview", args: { action: "push", repo: "owned/repo", folder: "/owned", branch: "reviewed-branch", expected_remote_sha: "a".repeat(40) } });
  expect(confirm.hasAttribute("disabled")).toBe(true); expect(effect).not.toHaveBeenCalled();
  expect(screen.getByText("Account: @owned · github.com")).toBeTruthy();
  fireEvent.click(screen.getByRole("checkbox")); fireEvent.click(confirm);
  await waitFor(() => expect(effect).toHaveBeenCalledTimes(1));
  expect(effect).toHaveBeenCalledWith({ kind: "files", action: "git_confirm", args: { ticket: "native-push" } });
});

test("clone reviews a new relative destination and opens only the native completed path", async () => {
  const run = vi.fn().mockResolvedValue({ ticket: "native-clone", preview: { repository: "owned/repo", folder: "/owned", action: "clone", destination: "/owned/new", remote_branch: "refs/heads/main", expected_remote_sha: "a".repeat(40), recovery_path: "/owned/.ag1-clone-native", account: { login: "owned", host: "github.com" } } });
  const effect = vi.fn().mockResolvedValue({ ok: true, result: { operation: "clone", outcome: "applied", path: "/owned/new" } }), attach = vi.fn();
  render(<GitHubLocal repo="owned/repo" workspace="/owned" run={run} effect={effect} initialAction="clone" attach={attach} />);
  fireEvent.change(screen.getByLabelText("Git remote branch"), { target: { value: "main" } });
  fireEvent.change(screen.getByLabelText("Reviewed remote SHA"), { target: { value: "a".repeat(40) } });
  fireEvent.change(screen.getByLabelText("Clone destination"), { target: { value: "new" } });
  fireEvent.click(screen.getByText("Review local Git request"));
  const confirm = await screen.findByText("Confirm local Git request");
  expect(run).toHaveBeenCalledWith({ kind: "files", action: "git_preview", args: { action: "clone", repo: "owned/repo", folder: "/owned", branch: "main", expected_remote_sha: "a".repeat(40), destination: "new" } });
  expect(screen.getByText("/owned/new")).toBeTruthy();expect(screen.getByText("/owned/.ag1-clone-native")).toBeTruthy();
  expect(screen.getByText("Exact local Git details").closest("details")?.open).toBe(false);
  expect(effect).not.toHaveBeenCalled();expect(attach).not.toHaveBeenCalled();
  fireEvent.click(confirm);fireEvent.click(await screen.findByText("Open cloned project"));
  expect(attach).toHaveBeenCalledWith("/owned/new");
});

test("uncertain clone retains the native recovery location without claiming completion", async () => {
  const run = vi.fn().mockResolvedValue({ ticket: "native-clone", preview: { repository: "owned/repo", folder: "/owned", action: "clone" } });
  const effect = vi.fn().mockResolvedValue({ ok: false, outcome: "unknown", recovery_path: "/owned/.ag1-clone-retained" });
  render(<GitHubLocal repo="owned/repo" workspace="/owned" run={run} effect={effect} initialAction="clone" attach={vi.fn()} />);
  fireEvent.click(screen.getByText("Review local Git request"));fireEvent.click(await screen.findByText("Confirm local Git request"));
  expect((await screen.findByRole("alert")).textContent).toContain("/owned/.ag1-clone-retained");
  expect(screen.queryByText("Open cloned project")).toBeNull();expect(effect).toHaveBeenCalledTimes(1);
});

test("PR acquisition and checkout require two separate native confirmations", async () => {
  const sha = "b".repeat(40);
  const run = vi.fn().mockImplementation(async ({ args }) => ({ ticket: `native-${args.action}`, preview: { action: args.action, repository: "owned/repo", folder: "/owned", target_branch: args.branch, target_sha: args.target_sha, remote_branch: "refs/pull/7/head" } }));
  const effect = vi.fn().mockResolvedValueOnce({ ok: true, result: { operation: "fetch_pr", outcome: "applied", pull_number: 7, commit: sha } }).mockResolvedValueOnce({ ok: true, result: { operation: "checkout", outcome: "applied", branch: "refs/heads/pr/7", commit: sha, recovery_path: "/owned/.ag1-git-recovery-native" } });
  render(<GitHubLocal repo="owned/repo" workspace="/owned" run={run} effect={effect} initialAction="fetch_pr" initialPullNumber={7} initialSha={sha} />);
  fireEvent.click(screen.getByText("Review local Git request"));
  const download = await screen.findByText("Confirm local Git request");
  expect(run).toHaveBeenCalledWith({ kind: "files", action: "git_preview", args: { action: "fetch_pr", repo: "owned/repo", folder: "/owned", pull_number: 7, expected_remote_sha: sha } });
  expect(effect).not.toHaveBeenCalled();fireEvent.click(download);
  await screen.findByText(/Pull request objects fetched/);
  expect(effect).toHaveBeenCalledTimes(1);expect((screen.getByLabelText("Checkout branch") as HTMLInputElement).value).toBe("pr/7");
  expect((screen.getByLabelText("Checkout target SHA") as HTMLInputElement).value).toBe(sha);
  fireEvent.click(screen.getByText("Review local Git request"));
  const checkout = await screen.findByText("Confirm local Git request");
  expect(run).toHaveBeenLastCalledWith({ kind: "files", action: "git_preview", args: { action: "checkout", repo: "owned/repo", folder: "/owned", branch: "pr/7", target_sha: sha } });
  expect(effect).toHaveBeenCalledTimes(1);fireEvent.click(checkout);
  await screen.findByText(/Checked out refs\/heads\/pr\/7/);expect(effect).toHaveBeenLastCalledWith({ kind: "files", action: "git_confirm", args: { ticket: "native-checkout" } });
});

test("pull downloads first and requires a fresh exact fast-forward confirmation", async () => {
  const sha = "c".repeat(40);
  const run = vi.fn().mockImplementation(async ({ args }) => ({ ticket: `native-${args.action}`, preview: { action: args.action, repository: "owned/repo", folder: "/owned", target_branch: "refs/heads/main", target_sha: sha } }));
  const effect = vi.fn().mockResolvedValueOnce({ ok: true, result: { operation: "fetch", outcome: "applied", repository: "owned/repo", remote_branch: "refs/heads/main", commit: sha } }).mockResolvedValueOnce({ ok: true, result: { operation: "pull", outcome: "applied", branch: "refs/heads/main", commit: sha, recovery_path: "/owned/.ag1-git-recovery-native" } });
  render(<GitHubLocal repo="owned/repo" workspace="/owned" run={run} effect={effect} />);
  fireEvent.change(screen.getByLabelText("Local Git operation"), { target: { value: "pull" } });
  fireEvent.change(screen.getByLabelText("Git remote branch"), { target: { value: "main" } });
  fireEvent.change(screen.getByLabelText("Reviewed remote SHA"), { target: { value: sha } });
  fireEvent.click(screen.getByText("Review local Git request"));
  const download = await screen.findByText("Confirm local Git request");
  expect(run).toHaveBeenLastCalledWith({ kind: "files", action: "git_preview", args: { action: "fetch", repo: "owned/repo", folder: "/owned", branch: "main", expected_remote_sha: sha } });
  expect(effect).not.toHaveBeenCalled(); fireEvent.click(download);
  await screen.findByText(/Step 2 of 2: fetch completed/);
  expect(effect).toHaveBeenCalledTimes(1); expect(run).toHaveBeenCalledTimes(1);
  fireEvent.click(screen.getByText("Review local Git request"));
  const advance = await screen.findByText("Confirm local Git request");
  expect(run).toHaveBeenLastCalledWith({ kind: "files", action: "git_preview", args: { action: "pull", repo: "owned/repo", folder: "/owned", branch: "main", expected_remote_sha: sha } });
  expect(effect).toHaveBeenCalledTimes(1); fireEvent.click(advance);
  await screen.findByText(/Fast-forwarded refs\/heads\/main/);
  expect(effect).toHaveBeenLastCalledWith({ kind: "files", action: "git_confirm", args: { ticket: "native-pull" } });
});

test("editing a fetched pull target returns to download review and dismisses its plan", async () => {
  const sha = "d".repeat(40);
  const run = vi.fn().mockImplementation(async ({ args }) => ({ ticket: `native-${args.action}`, preview: { action: args.action, repository: "owned/repo", folder: "/owned" } }));
  const effect = vi.fn().mockResolvedValue({ ok: true, result: { operation: "fetch", outcome: "applied", repository: "owned/repo", remote_branch: "refs/heads/main", commit: sha } });
  render(<GitHubLocal repo="owned/repo" workspace="/owned" run={run} effect={effect} />);
  fireEvent.change(screen.getByLabelText("Local Git operation"), { target: { value: "pull" } });
  fireEvent.change(screen.getByLabelText("Git remote branch"), { target: { value: "main" } });
  fireEvent.change(screen.getByLabelText("Reviewed remote SHA"), { target: { value: sha } });
  fireEvent.click(screen.getByText("Review local Git request")); fireEvent.click(await screen.findByText("Confirm local Git request"));
  await screen.findByText(/Step 2 of 2/);
  fireEvent.click(screen.getByText("Review local Git request")); await screen.findByText("Confirm local Git request");
  fireEvent.change(screen.getByLabelText("Reviewed remote SHA"), { target: { value: "e".repeat(40) } });
  expect(run).toHaveBeenLastCalledWith({ kind: "files", action: "git_dismiss", args: { ticket: "native-pull" } });
  expect(screen.queryByText("Confirm local Git request")).toBeNull(); expect(screen.getByText(/Step 1 of 2/)).toBeTruthy();
  fireEvent.click(screen.getByText("Review local Git request")); await screen.findByText("Confirm local Git request");
  expect(run).toHaveBeenLastCalledWith({ kind: "files", action: "git_preview", args: { action: "fetch", repo: "owned/repo", folder: "/owned", branch: "main", expected_remote_sha: "e".repeat(40) } });
  expect(effect).toHaveBeenCalledTimes(1);
});

test("recovery inspection grants no repair and an explicit choice needs fresh confirmation", async () => {
  const id = ".ag1-git-recovery-" + "a".repeat(32);
  const run = vi.fn().mockImplementation(async ({ args }) => ({ ticket: `native-${args.action}`, preview: { action: args.action, repository: "owned/repo", folder: "/owned", choice: args.choice, recovery_warning: "Later edits and changed locks are refused." } }));
  const effect = vi.fn().mockResolvedValueOnce({ ok: true, result: { operation: "recovery_status", outcome: "pending", recovery_id: id, can_restore: true } }).mockResolvedValueOnce({ ok: true, result: { operation: "recover", outcome: "applied", recovery_choice: "restore", branch: "refs/heads/main", commit: "b".repeat(40) } });
  render(<GitHubLocal repo="owned/repo" workspace="/owned" run={run} effect={effect} />);
  fireEvent.change(screen.getByLabelText("Local Git operation"), { target: { value: "recovery_status" } });
  fireEvent.click(screen.getByText("Review local Git request"));
  fireEvent.click(await screen.findByText("Confirm local Git request"));
  await screen.findByLabelText("Git recovery choice");
  expect(effect).toHaveBeenCalledTimes(1); expect(run).toHaveBeenCalledTimes(1);
  expect(screen.getByText("Review local Git request").hasAttribute("disabled")).toBe(true);
  fireEvent.change(screen.getByLabelText("Git recovery choice"), { target: { value: "restore" } });
  fireEvent.click(screen.getByText("Review local Git request"));
  const confirm = await screen.findByText("Confirm local Git request");
  expect(run).toHaveBeenLastCalledWith({ kind: "files", action: "git_preview", args: { action: "recover", repo: "owned/repo", folder: "/owned", recovery_id: id, choice: "restore" } });
  expect(screen.getByText("Recovery choice: restore recorded starting state")).toBeTruthy(); expect(effect).toHaveBeenCalledTimes(1);
  fireEvent.click(confirm); await screen.findByText(/Recorded starting state restored/);
  expect(effect).toHaveBeenLastCalledWith({ kind: "files", action: "git_confirm", args: { ticket: "native-recover" } });
});

test("an interrupted repair only offers finish and preserves uncertainty without automatic retry", async () => {
  const id = ".ag1-git-recovery-" + "a".repeat(32);
  const run = vi.fn().mockImplementation(async ({ args }) => ({ ticket: `native-${args.action}`, preview: { action: args.action, repository: "owned/repo", folder: "/owned" } }));
  const effect = vi.fn().mockResolvedValueOnce({ ok: true, result: { operation: "recovery_status", outcome: "pending", recovery_id: id, can_restore: false } }).mockResolvedValueOnce({ ok: false, outcome: "unknown", recovery_path: `/owned/${id}` });
  render(<GitHubLocal repo="owned/repo" workspace="/owned" run={run} effect={effect} />);
  fireEvent.change(screen.getByLabelText("Local Git operation"), { target: { value: "recovery_status" } });
  fireEvent.click(screen.getByText("Review local Git request")); fireEvent.click(await screen.findByText("Confirm local Git request"));
  const choice = await screen.findByLabelText("Git recovery choice");
  expect(screen.queryByRole("option", { name: "Restore the recorded starting state" })).toBeNull();
  fireEvent.change(choice, { target: { value: "finish" } }); fireEvent.click(screen.getByText("Review local Git request")); fireEvent.click(await screen.findByText("Confirm local Git request"));
  expect((await screen.findByRole("alert")).textContent).toContain("uncertain");
  expect(effect).toHaveBeenCalledTimes(2); expect(screen.queryByText(/Retained target recovered/)).toBeNull();
  fireEvent.click(screen.getByText("Inspect recovery")); expect(effect).toHaveBeenCalledTimes(2); expect(run).toHaveBeenCalledTimes(2);
});
