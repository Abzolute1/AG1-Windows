import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import { GitHubUpload } from "./GitHubUpload";
const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
afterEach(() => { cleanup(); vi.useRealTimers(); invoke.mockReset(); });
const preview = { confirmation: "owned-upload", account: { login: "owned", host: "github.com" }, request: { id: "releases.upload_asset", endpoint: "repos/owned/repo/releases/3/assets?name=owned.bin" }, upload_source: { path: "dist/owned.bin", bytes: 5, sha256: "a".repeat(64) }, risk: "consequential", expires_in_seconds: 300 };
function fill() { fireEvent.change(screen.getByLabelText("File path"), { target: { value: "dist/owned.bin" } }); fireEvent.change(screen.getByLabelText("Asset filename"), { target: { value: "owned.bin" } }); }
test("native file selection and upload require separate approval of exact bytes", async () => {
  const run = vi.fn().mockResolvedValue(preview); invoke.mockResolvedValue({ ok: true, value: { data: { id: 99 } } });
  render(<GitHubUpload repo="owned/repo" release={3} workspace="/owned" run={run} />); expect(run).not.toHaveBeenCalled(); fill(); fireEvent.click(screen.getByText("Review file upload")); await screen.findByLabelText("Reviewed upload file");
  expect(run).toHaveBeenCalledWith({ kind: "files", action: "github_upload_preview", args: { operation: "releases.upload_asset", repo: "owned/repo", parameters: { release_id: 3, folder: "/owned", path: "dist/owned.bin", name: "owned.bin", content_type: "application/octet-stream" } } });
  const confirm = screen.getByText("Confirm file upload") as HTMLButtonElement; expect(confirm.disabled).toBe(true); expect(invoke).not.toHaveBeenCalled(); fireEvent.click(screen.getByRole("checkbox")); fireEvent.click(confirm); await screen.findByText(/GitHub accepted the upload/); expect(invoke).toHaveBeenCalledWith("github_command", { action: "api_confirm", args: { confirmation: "owned-upload" } });
});
test("late preview after scope changes is dismissed", async () => {
  let resolve!: (v: unknown) => void; const run = vi.fn(() => new Promise<any>(r => { resolve = r; })); invoke.mockResolvedValue({ ok: true });
  const view = render(<GitHubUpload repo="owned/repo" release={3} workspace="/owned" run={run} />); fill(); fireEvent.click(screen.getByText("Review file upload")); view.rerender(<GitHubUpload repo="other/repo" release={3} workspace="/owned" run={run} />); await act(async () => { resolve(preview); }); expect(screen.queryByText("Confirm file upload")).toBeNull(); expect(invoke).toHaveBeenCalledWith("github_command", { action: "api_dismiss", args: { confirmation: "owned-upload" } });
});
test("uncertain upload consumes preview and cannot be retried implicitly", async () => {
  const run = vi.fn().mockResolvedValue(preview); invoke.mockResolvedValue({ ok: false, error: { outcome: "unknown", message: "Interrupted" } });
  render(<GitHubUpload repo="owned/repo" release={3} workspace="/owned" run={run} />); fill(); fireEvent.click(screen.getByText("Review file upload")); await screen.findByRole("checkbox"); fireEvent.click(screen.getByRole("checkbox")); fireEvent.click(screen.getByText("Confirm file upload")); await screen.findByText(/may have left an asset/); expect(screen.queryByText("Confirm file upload")).toBeNull(); expect(invoke.mock.calls.filter(c => c[1].action === "api_confirm")).toHaveLength(1);
});
test("expired upload requires a fresh native preview", async () => {
  invoke.mockResolvedValue({ ok: true });
  const run = vi.fn().mockResolvedValue({ ...preview, expires_in_seconds: 1 }); render(<GitHubUpload repo="owned/repo" release={3} workspace="/owned" run={run} />); fill(); fireEvent.click(screen.getByText("Review file upload")); await screen.findByRole("checkbox"); fireEvent.click(screen.getByRole("checkbox")); await waitFor(() => expect((screen.getByText("Confirm file upload") as HTMLButtonElement).disabled).toBe(true), { timeout: 2000 }); expect(invoke).not.toHaveBeenCalled();
});
