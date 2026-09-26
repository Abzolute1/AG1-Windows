import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { webcrypto } from "node:crypto";
import { useState } from "react";
import { ReviewPanel, boundedPanelWidth, restoreReviewFocus, type ReviewTab } from "./ReviewPanel";
import { artifactBytes, artifactPage, type Artifact } from "./artifactData";
import { ArtifactsPane } from "./ArtifactsPane";
import { diffFiles, WorkspaceReview, DIFF_FILES_PER_PAGE, DIFF_ROWS_PER_PAGE, DIFF_SOURCE_PAGE } from "./WorkspaceReview";
import type { JsonValue } from "./generated/protocol";

beforeEach(() => {
  vi.stubGlobal("crypto", webcrypto);
  vi.stubGlobal("matchMedia", () => ({ matches: false }));
  URL.createObjectURL = vi.fn(() => "blob:retained-output"); URL.revokeObjectURL = vi.fn();
});
afterEach(() => { cleanup(); vi.unstubAllGlobals(); });
async function artifact(text = "<script>not executable</script>", kind = "saved-file", mime_type = "text/plain"): Promise<Artifact> {
  const bytes = new TextEncoder().encode(text);
  const sha256 = Array.from(new Uint8Array(await crypto.subtle.digest("SHA-256", bytes)), b => b.toString(16).padStart(2, "0")).join("");
  return { id: "artifact-1", kind, mime_type, bytes: bytes.length, sha256, created_at: "2026-09-10T00:00:00Z" };
}
const page = (text: string) => ({ base64: btoa(text), next: null });

test("bounded panel width rejects invalid persisted values", () => {
  expect([NaN, Infinity, -40, 5000, 600].map(boundedPanelWidth)).toEqual([520, 520, 320, 960, 600]);
});
test("diff counts reflect hunk lines, including source starting with +++ or ---", () => {
  const files = diffFiles("HEAD abc\n M file\0\ndiff --git a/file b/file\n--- a/file\n+++ b/file\n@@ -1 +1 @@\n---old\n+++new\n");
  expect(files).toHaveLength(1); expect(files[0]).toMatchObject({ additions: 1, deletions: 1 });
});
test("artifact bytes enforce paging, complete length, digest and cancellation", async () => {
  const a = await artifact("hello");
  expect(new TextDecoder().decode(await artifactBytes(async () => page("hello"), a))).toBe("hello");
  await expect(artifactBytes(async () => page("other"), a)).rejects.toThrow("integrity mismatch");
  await expect(artifactBytes(async () => page("he"), a)).rejects.toThrow("invalid artifact byte page");
  await expect(artifactBytes(async () => ({ base64: btoa("he"), next: "0" }), a)).rejects.toThrow("invalid artifact byte page");
  const run = vi.fn(async () => page("hello"));
  await expect(artifactBytes(run, a, () => false)).rejects.toThrow("selection changed"); expect(run).not.toHaveBeenCalled();
  await expect(artifactBytes(run, { ...a, bytes: 9 * 1024 * 1024 })).rejects.toThrow("8 MB");
});
test("empty artifacts download correctly and invalid page sizes are rejected", async () => {
  expect(await artifactBytes(async () => page(""), await artifact(""))).toHaveLength(0);
  await expect(artifactPage(async () => page("a".repeat(32769)), { ...await artifact(), bytes: 32769 }, 0)).rejects.toThrow("invalid artifact byte page");
});
test("artifact preview is literal, lazily read, and download uses inert original bytes", async () => {
  const text = "<svg><script>alert(1)</script></svg>", a = await artifact(text, "drawing", "image/svg+xml");
  const run = vi.fn(async (c: any): Promise<JsonValue> => c.kind === "artifacts" ? { artifacts: [a], next: null } as any : page(text));
  const view = render(<ArtifactsPane run={run} />);
  fireEvent.click(await screen.findByRole("button", { name: /drawing/ }));
  await screen.findByText(text);
  expect(document.querySelector(".artifact-inspector svg")).not.toBeNull(); // Copy icon only; source is text.
  expect(document.querySelector("script, iframe")).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "Prepare download" }));
  expect((await screen.findByRole("link", { name: "Download original" })).getAttribute("download")).toMatch(/\.svg$/);
  const blob = vi.mocked(URL.createObjectURL).mock.calls[0][0] as Blob; expect(blob.type).toBe("application/octet-stream"); expect(blob.size).toBe(a.bytes);
  view.unmount(); expect(URL.revokeObjectURL).toHaveBeenCalledWith("blob:retained-output");
});
test("switching artifact selections discards late source reads", async () => {
  const a = await artifact("old"), b = { ...await artifact("new", "new-output"), id: "artifact-2" };
  let resolve!: (value: JsonValue) => void;
  const run = vi.fn(async (c: any): Promise<JsonValue> => c.kind === "artifacts" ? { artifacts: [a, b], next: null } as any : c.artifact === a.id ? new Promise(done => { resolve = done; }) : page("new"));
  render(<ArtifactsPane run={run} />);
  fireEvent.click(await screen.findByRole("button", { name: /saved file/ }));
  fireEvent.click(screen.getByRole("button", { name: /new output/ }));
  await screen.findByText("new");
  await act(async () => resolve(page("old")));
  expect(screen.queryByText("old")).toBeNull();
});
test("diagnostics are opt-in and artifact filtering describes only loaded records", async () => {
  const a = await artifact("{}", "context-capture");
  render(<ArtifactsPane run={async () => ({ artifacts: [a], next: null }) as any} />);
  await screen.findByText("No output artifacts on the loaded pages.");
  expect(screen.queryByRole("button", { name: /context capture/ })).toBeNull();
  fireEvent.click(screen.getByRole("checkbox", { name: "Include diagnostic records" }));
  expect(screen.getByRole("button", { name: /context capture/ })).toBeTruthy();
  fireEvent.change(screen.getByRole("textbox", { name: "Filter artifacts" }), { target: { value: "missing" } });
  expect(screen.getByText("No loaded artifacts match this filter.")).toBeTruthy();
});
test("panel retains source selection across tab and close changes; keyboard resize is bounded", async () => {
  const a = await artifact("retained");
  const run = vi.fn(async (c: any): Promise<JsonValue> => c.kind === "artifacts" ? { artifacts: [a], next: null } as any : c.kind === "artifact_read" ? page("retained") : { view: "HEAD test" });
  function Harness() {
    const [tab, setTab] = useState<ReviewTab>("artifacts"), [open, setOpen] = useState(true);
    return <><button onClick={() => setOpen(true)}>Reopen</button><ReviewPanel open={open} tab={tab} onTab={setTab} run={run} onClose={() => setOpen(false)} /></>;
  }
  render(<Harness />);
  fireEvent.click(await screen.findByRole("button", { name: /saved file/ })); await screen.findByText("retained");
  const resize = screen.getByRole("separator"); fireEvent.keyDown(resize, { key: "Home" }); expect(resize.getAttribute("aria-valuenow")).toBe("320");
  fireEvent.keyDown(resize, { key: "ArrowRight" }); expect(resize.getAttribute("aria-valuenow")).toBe("320");
  fireEvent.keyDown(resize, { key: "ArrowLeft" }); expect(resize.getAttribute("aria-valuenow")).toBe("352");
  fireEvent.click(screen.getByRole("button", { name: "Review" }));
  await screen.findByText("No tracked changes against HEAD.");
  fireEvent.click(screen.getByRole("button", { name: "Artifacts" }));
  fireEvent.click(screen.getByRole("button", { name: "Close review panel" }));
  expect(screen.queryByRole("complementary", { name: "Review and artifacts" })).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "Reopen" }));
  expect(screen.getByText("retained")).toBeTruthy();
  expect(run.mock.calls.filter(([c]) => c.kind === "artifact_read")).toHaveLength(1);
});
test("closing during a catalog read does not permanently disable refresh", async () => {
  let resolve!: (value: JsonValue) => void;
  const run = vi.fn(() => new Promise<JsonValue>(done => { resolve = done; }));
  const view = render(<ArtifactsPane visible run={run} />);
  view.rerender(<ArtifactsPane visible={false} run={run} />);
  await act(async () => resolve({ artifacts: [], next: null }));
  view.rerender(<ArtifactsPane visible run={run} />);
  await waitFor(() => expect(screen.getByRole<HTMLButtonElement>("button", { name: "Refresh outputs" }).disabled).toBe(false));
});

test("large diffs page files, rows and raw source without losing later content", async () => {
  const diff = Array.from({ length: 130 }, (_, i) => `diff --git a/file-${i} b/file-${i}\n--- a/file-${i}\n+++ b/file-${i}\n@@ -1 +1,700 @@\n-old\n${Array.from({ length: 700 }, (_, j) => `+line-${j}`).join("\n")}\n`).join("");
  render(<WorkspaceReview run={async () => ({ view: diff })} />);
  fireEvent.click(await screen.findByRole("button", { name: /a\/file-0 b\/file-0/ }));
  expect(document.querySelectorAll(".review-file-list button")).toHaveLength(DIFF_FILES_PER_PAGE);
  expect(document.querySelectorAll(".review-diff pre > span")).toHaveLength(DIFF_ROWS_PER_PAGE);
  expect(document.querySelector(".review-metadata pre")!.textContent!.length).toBe(DIFF_SOURCE_PAGE);
  fireEvent.click(screen.getByRole("button", { name: "Next diff page" }));
  expect(document.querySelector(".review-diff pre")!.textContent).toContain("+line-594");
  fireEvent.click(screen.getByRole("button", { name: "Next files" }));
  expect(document.querySelectorAll(".review-file-list button")).toHaveLength(30);
  fireEvent.change(screen.getByRole("textbox", { name: "Filter changed files" }), { target: { value: "file-129" } });
  expect(document.querySelectorAll(".review-file-list button")).toHaveLength(1);
});

test("workspace changes clear across scopes and ignore late prior replies", async () => {
  let resolve!: (value: JsonValue) => void;
  const old = vi.fn(() => new Promise<JsonValue>(done => { resolve = done; }));
  const view = render(<WorkspaceReview run={old} />);
  view.rerender(<WorkspaceReview run={async () => ({ view: "HEAD new" })} />);
  await screen.findByText("No tracked changes against HEAD.");
  await act(async () => resolve({ view: "diff --git a/private b/private\n+private" }));
  expect(screen.queryByText(/a\/private/)).toBeNull();
  expect(screen.getByRole<HTMLButtonElement>("button", { name: "Refresh changes" }).disabled).toBe(false);
});

test("artifact scope changes remove selected bytes, downloads and diagnostics", async () => {
  const a = await artifact("private");
  let resolve!: (value: JsonValue) => void;
  const old = vi.fn(async (c: any): Promise<JsonValue> => c.kind === "artifacts" ? { artifacts: [a], next: null } as any : new Promise(done => { resolve = done; }));
  const view = render(<ArtifactsPane run={old} />);
  fireEvent.click(await screen.findByRole("button", { name: /saved file/ }));
  fireEvent.click(screen.getByRole("checkbox", { name: "Include diagnostic records" }));
  view.rerender(<ArtifactsPane run={async () => ({ artifacts: [], next: null })} />);
  await screen.findByText("No output artifacts on the loaded pages.");
  await act(async () => resolve(page("private")));
  expect(screen.queryByText("private")).toBeNull();
  expect(screen.queryByRole("region", { name: "Selected artifact" })).toBeNull();
  expect(screen.getByRole<HTMLInputElement>("checkbox", { name: "Include diagnostic records" }).checked).toBe(false);
  expect(URL.createObjectURL).not.toHaveBeenCalled();
});

test("late catalog replies cannot expose artifacts from a previous scope", async () => {
  const a = await artifact();
  let resolve!: (value: JsonValue) => void;
  const view = render(<ArtifactsPane run={() => new Promise(done => { resolve = done; })} />);
  view.rerender(<ArtifactsPane run={async () => ({ artifacts: [], next: null })} />);
  await screen.findByText("No output artifacts on the loaded pages.");
  await act(async () => resolve({ artifacts: [a], next: null } as any));
  expect(screen.queryByRole("button", { name: /saved file/ })).toBeNull();
});

test("focus falls back to the header when a diagnostic opener has unmounted", () => {
  const opener = document.createElement("button");
  const { unmount } = render(<><button aria-label="Artifacts" /><textarea aria-label="Message AG1" /></>);
  const fallback = screen.getByRole("button", { name: "Artifacts" });
  vi.spyOn(fallback, "getClientRects").mockReturnValue([{}] as any);
  restoreReviewFocus(opener, "artifacts");
  expect(document.activeElement).toBe(fallback);
  unmount();
});

test("unavailable Git helper produces actionable review text", async () => {
  render(<WorkspaceReview run={async () => { throw new Error("unavailable: Configure the native Git helper"); }} />);
  expect((await screen.findByRole("alert")).textContent).toContain("Git helper is not configured. Artifact review is still available.");
});

test("native effect custody and executor records stay behind diagnostics", async () => {
  const records = await Promise.all(["native-change-bytes", "tool-arguments", "tool-native-result", "tool-post-hook-result", "tool-request-result"].map(async kind => ({ ...await artifact("{}", kind), id: kind })));
  render(<ArtifactsPane run={async () => ({ artifacts: records, next: null }) as any} />);
  await screen.findByText("No output artifacts on the loaded pages.");
  fireEvent.click(screen.getByRole("checkbox", { name: "Include diagnostic records" }));
  expect(document.querySelectorAll(".artifact-list button")).toHaveLength(5);
});

test("native diff pages preserve the snapshot digest across the bridge limit", async () => {
  const diff = "diff --git a/large b/large\n--- a/large\n+++ b/large\n@@ -1 +1,3000 @@\n-old\n" + "+large changed line\n".repeat(3000);
  const bytes = new TextEncoder().encode(diff), a = await artifact(diff);
  const run = vi.fn(async (c: any) => {
    const offset = Number(c.args.offset || 0), end = Math.min(bytes.length, offset + 24000);
    if (offset) expect(c.args.sha256).toBe(a.sha256);
    return { base64: btoa(String.fromCharCode(...bytes.slice(offset, end))), next: end === bytes.length ? null : String(end), bytes: bytes.length, sha256: a.sha256 };
  });
  render(<WorkspaceReview run={run} />);
  fireEvent.click(await screen.findByRole("button", { name: /a\/large b\/large/ }));
  expect(run.mock.calls.length).toBe(3);
  expect(screen.getByText("+3000")).toBeTruthy();
});
