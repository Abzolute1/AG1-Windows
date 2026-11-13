import { afterEach, expect, test, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { SiteIcons, SourceList, webSources } from "./WebSources";
import type { ClientMessage } from "./generated/protocol";
afterEach(() => { cleanup(); vi.useRealTimers(); });
const message: ClientMessage = { id: "result", role: "toolResult", tool: { name: "webSearch", call_id: "call", failed: false }, blocks: [{ kind: "unknown", value: { kind: "web_result", sources: [{ url: "https://example.com/a", title: "First page", description: "A short result summary." }, { url: "https://example.com/b", title: "Second page" }, { url: "javascript:alert(1)", title: "Bad" }] } }] };
test("source URLs are validated, icons deduplicate and use only native raster data", async () => {
  const run = vi.fn(async () => ({ dataUrl: "data:image/png;base64,aGVsbG8=" }));
  const sources = webSources(message); expect(sources).toHaveLength(2);
  const { container } = render(<><SiteIcons sources={sources} run={run} /><SourceList sources={sources} run={run} /></>);
  await waitFor(() => expect(container.querySelectorAll('img')).toHaveLength(3));
  expect(run).toHaveBeenCalledTimes(1);
  expect(run).toHaveBeenCalledWith({ kind: "runtime", action: "website_icon", args: { entry: "result", url: "https://example.com/a" } });
  expect(screen.getByRole('link', { name: /First page/ }).getAttribute('href')).toBe('https://example.com/a');
  expect(screen.getByText("A short result summary.")).toBeTruthy();
  fireEvent.error(container.querySelector('img')!); expect(container.querySelectorAll('img')).toHaveLength(2);
  expect(webSources({ ...message, tool: { ...message.tool!, failed: true } })).toEqual([]);
});
test("failed or unsafe icon responses leave an offline-friendly fallback", async () => {
  const run = vi.fn(async () => ({ dataUrl: "https://tracker.example/icon.svg" }));
  const { container } = render(<SiteIcons sources={webSources(message)} run={run} />);
  await waitFor(() => expect(run).toHaveBeenCalledTimes(1));
  expect(container.querySelector('img')).toBeNull();expect(container.querySelector('.site-icon svg')).toBeTruthy();
});

test("more than four visible hosts queue and every icon eventually resolves", async () => {
  const pending: Array<() => void> = [];
  let active = 0, peak = 0;
  const run = vi.fn(() => new Promise<{ dataUrl: string }>(resolve => {
    active++; peak = Math.max(peak, active);
    pending.push(() => { active--; resolve({ dataUrl: "data:image/png;base64,aGVsbG8=" }); });
  }));
  const sources = Array.from({ length: 12 }, (_, i) => ({ entry: `entry-${i}`, url: `https://site${i}.example/a`, title: `Site ${i}`, description: "", host: `site${i}.example` }));
  const { container } = render(<SourceList sources={sources} run={run} />);
  await waitFor(() => expect(run).toHaveBeenCalledTimes(4));
  for (let batch = 0; batch < 3; batch++) {
    await act(async () => { pending.splice(0).forEach(done => done()); });
    await waitFor(() => expect(container.querySelectorAll("img")).toHaveLength((batch + 1) * 4));
  }
  expect(run).toHaveBeenCalledTimes(12); expect(peak).toBe(4);
});

test("a temporary miss retries and replaces the fallback without reopening chat", async () => {
  vi.useFakeTimers();
  const run = vi.fn().mockResolvedValueOnce({ dataUrl: null }).mockResolvedValue({ dataUrl: "data:image/png;base64,aGVsbG8=" });
  const { container } = render(<SiteIcons sources={webSources(message)} run={run} />);
  await act(async () => { await vi.advanceTimersByTimeAsync(0); });
  expect(run).toHaveBeenCalledTimes(1); expect(container.querySelector("img")).toBeNull();
  await act(async () => { await vi.advanceTimersByTimeAsync(30_100); });
  expect(run).toHaveBeenCalledTimes(2); expect(container.querySelector("img")).not.toBeNull();
});

test("offline failures recover on reconnect and unmounted icons do not retry", async () => {
  vi.useFakeTimers();
  const run = vi.fn().mockRejectedValueOnce(new Error("offline")).mockResolvedValue({ dataUrl: "data:image/png;base64,aGVsbG8=" });
  const { container, unmount } = render(<SiteIcons sources={webSources(message)} run={run} />);
  await act(async () => { await vi.advanceTimersByTimeAsync(0); });
  await act(async () => { window.dispatchEvent(new Event("online")); });
  expect(container.querySelector("img")).not.toBeNull();
  unmount();
  await act(async () => { await vi.advanceTimersByTimeAsync(120_000); });
  expect(run).toHaveBeenCalledTimes(2);
});
