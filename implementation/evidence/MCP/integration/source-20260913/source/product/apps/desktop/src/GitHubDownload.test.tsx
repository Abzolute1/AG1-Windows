import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import { webcrypto, createHash } from "node:crypto";
import { GitHubDownload } from "./GitHubDownload";
const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
const bytes = new Uint8Array([0, 255, 13, 10, 65]), value = { id: "download-owned", name: "owned.bin", bytes: 5, sha256: createHash("sha256").update(bytes).digest("hex"), mime: "application/octet-stream", operation: "releases.download_asset" };
function result(data: unknown) { return { ok: true, value: { data } }; }
function page(offset: number, data = bytes.slice(offset, offset + 3)) { return result({ download: value, offset, next_offset: offset + data.length, complete: offset + data.length === 5, base64: Buffer.from(data).toString("base64") }); }
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); invoke.mockReset(); });
function browser() { vi.stubGlobal("crypto", webcrypto); const create = vi.fn((_blob: Blob) => "blob:owned"), revoke = vi.fn(); vi.stubGlobal("URL", class extends URL { static createObjectURL = create; static revokeObjectURL = revoke; }); const click = vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(() => {}); return { create, revoke, click }; }
test("download bytes are paged, digest verified and saved only on explicit click", async () => {
  const b = browser(); invoke.mockImplementation(async (_, { args }) => args.operation === "downloads.read" ? page(args.parameters.offset) : result({ download: value }));
  render(<GitHubDownload scope="one" request={{ operation: "releases.download_asset", repo: "owned/repo", parameters: { asset_id: 3 } }} />);
  expect(invoke).not.toHaveBeenCalled(); fireEvent.click(screen.getByText("Download file")); await screen.findByText("Save downloaded file"); expect(b.click).not.toHaveBeenCalled();
  fireEvent.click(screen.getByText("Save downloaded file")); await waitFor(() => expect(b.click).toHaveBeenCalledTimes(1)); expect(invoke.mock.calls.map(c => c[1].args.parameters.offset).filter(x => x !== undefined)).toEqual([0, 3]);
  const blob = b.create.mock.calls[0][0] as Blob; expect(blob.size).toBe(5); expect(blob.type).toBe("application/octet-stream");
});
test.each(["digest", "identity", "range", "permission"])("%s failure never saves a file", async failure => {
  const b = browser(); invoke.mockImplementation(async () => failure === "permission" ? { ok: false, error: { message: "Permission denied" } } : result({ download: failure === "identity" ? { ...value, id: "download-other" } : value, offset: 0, next_offset: failure === "range" ? 4 : 5, complete: true, base64: Buffer.from(failure === "digest" ? [1, 2, 3, 4, 5] : bytes).toString("base64") }));
  render(<GitHubDownload scope="one" value={value} />); fireEvent.click(screen.getByText("Save downloaded file")); await screen.findByRole("alert"); expect(b.click).not.toHaveBeenCalled(); expect(b.create).not.toHaveBeenCalled();
});
test.each(["scope", "disconnect", "cancel"])("late download page after %s cannot save or reveal bytes", async change => {
  const b = browser(); let resolve!: (value: unknown) => void; invoke.mockImplementation(() => new Promise(r => { resolve = r; }));
  const view = render(<GitHubDownload scope="one" value={value} />); fireEvent.click(screen.getByText("Save downloaded file"));
  if (change === "scope") view.rerender(<GitHubDownload scope="two" value={value} />);
  if (change === "disconnect") act(() => { window.dispatchEvent(new Event("ag1-github-disconnected")); });
  const finish = resolve;
  if (change === "cancel") fireEvent.click(screen.getByText("Cancel download"));
  await act(async () => { finish(page(0, bytes)); }); expect(b.click).not.toHaveBeenCalled(); expect(b.create).not.toHaveBeenCalled();
});
test("job log preview is bounded literal text", async () => {
  browser(); const data = Buffer.from("<script>literal</script>\nOwned log"); const log = { ...value, operation: "actions.download_job_log", name: "owned.txt", bytes: data.length };
  invoke.mockResolvedValue(result({ download: log, offset: 0, next_offset: data.length, complete: true, base64: data.toString("base64") }));
  const view = render(<GitHubDownload scope="one" value={log} />); fireEvent.click(screen.getByText("Preview log text")); await screen.findByText(/<script>literal/); expect(view.container.querySelector("script")).toBeNull(); expect(invoke.mock.calls[0][1].args.parameters.length).toBe(65536);
});
