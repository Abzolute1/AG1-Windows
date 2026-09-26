import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import { GitHubSecrets } from "./GitHubSecrets";
const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
afterEach(() => { cleanup(); invoke.mockReset(); });
const preview = { confirmation: "owned-secret", account: { login: "owned", host: "github.com" }, request: { id: "secrets.set", endpoint: "repos/owned/repo/actions/secrets/OWNED_SECRET", body: {} }, risk: "consequential", expires_in_seconds: 300 };
function fill() { fireEvent.change(screen.getByLabelText("Secret name"), { target: { value: "OWNED_SECRET" } }); }
test("secret values have no renderer input and native entry needs explicit target confirmation", async () => {
  invoke.mockImplementation((_c, p) => Promise.resolve(p.action === "api_preview" ? { ok: true, value: preview } : { ok: true, value: { data: { secret_saved: true } } }));
  render(<GitHubSecrets repo="owned/repo" scope="s" />); fill(); expect(document.querySelector('input[type="password"]')).toBeNull(); fireEvent.click(screen.getByText("Review secret destination")); await screen.findByLabelText("Reviewed secret destination");
  expect(invoke).toHaveBeenCalledWith("github_command", { action: "api_preview", args: { operation: "secrets.set", repo: "owned/repo", parameters: { target: "actions_repository", name: "OWNED_SECRET" } } });
  const confirm = screen.getByText("Open private secret entry") as HTMLButtonElement; expect(confirm.disabled).toBe(true); expect(invoke).toHaveBeenCalledTimes(1); fireEvent.click(screen.getByRole("checkbox")); fireEvent.click(confirm); await screen.findByText(/GitHub saved the secret/); expect(invoke).toHaveBeenLastCalledWith("github_command", { action: "api_confirm", args: { confirmation: "owned-secret" } });
});
test("organization secret access is explicit and does not imply selected repository scope", async () => {
  invoke.mockResolvedValue({ ok: true, value: preview }); render(<GitHubSecrets repo="owned/repo" scope="s" />); fireEvent.change(screen.getByLabelText("Secret target"), { target: { value: "actions_organization" } }); fireEvent.change(screen.getByLabelText("Organization"), { target: { value: "owned" } }); fill(); fireEvent.change(screen.getByLabelText("Repository IDs"), { target: { value: "42, 73" } }); fireEvent.click(screen.getByText("Review secret destination")); await screen.findByLabelText("Reviewed secret destination");
  expect(invoke).toHaveBeenCalledWith("github_command", { action: "api_preview", args: { operation: "secrets.set", parameters: { target: "actions_organization", organization: "owned", name: "OWNED_SECRET", visibility: "selected", selected_repository_ids: [42, 73] } } });
});
test("late secret previews are dismissed and uncertainty never resubmits a consumed confirmation", async () => {
  let resolve!: (v: unknown) => void; invoke.mockImplementation((_c, p) => p.action === "api_preview" ? new Promise(r => { resolve = r; }) : Promise.resolve({ ok: false, error: { message: "Interrupted", outcome: "unknown" } })); const view = render(<GitHubSecrets repo="owned/repo" scope="s" />); fill(); fireEvent.click(screen.getByText("Review secret destination")); view.rerender(<GitHubSecrets repo="owned/repo" scope="new" />); await act(async () => resolve({ ok: true, value: preview })); expect(screen.queryByLabelText("Reviewed secret destination")).toBeNull(); expect(invoke).toHaveBeenCalledWith("github_command", { action: "api_dismiss", args: { confirmation: "owned-secret" } });
  invoke.mockImplementation((_c, p) => Promise.resolve(p.action === "api_preview" ? { ok: true, value: preview } : { ok: false, error: { message: "Interrupted", outcome: "unknown" } })); fireEvent.click(screen.getByText("Review secret destination")); await screen.findByRole("checkbox"); fireEvent.click(screen.getByRole("checkbox")); fireEvent.click(screen.getByText("Open private secret entry")); await screen.findByText(/change may have applied/); expect(screen.queryByText("Open private secret entry")).toBeNull(); expect(invoke.mock.calls.filter(c => c[1].action === "api_confirm")).toHaveLength(1);
});
test("expired secret review cannot open native entry", async () => {
  invoke.mockResolvedValue({ ok: true, value: { ...preview, expires_in_seconds: 1 } }); render(<GitHubSecrets repo="owned/repo" scope="s" />); fill(); fireEvent.click(screen.getByText("Review secret destination")); await screen.findByRole("checkbox"); fireEvent.click(screen.getByRole("checkbox")); await waitFor(() => expect((screen.getByText("Open private secret entry") as HTMLButtonElement).disabled).toBe(true), { timeout: 2000 }); expect(invoke).toHaveBeenCalledTimes(1);
});
