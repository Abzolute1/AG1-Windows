import { StrictMode } from "react";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import { MCPDiscover } from "./MCPDiscover";
import { createDiscoverCache } from "./MCPDiscoverCache";
afterEach(cleanup);
const entry = (key: string, title: string, local = false) => ({ key, server: { name: `publisher/${key}`, title, description: "Literal <script>listing</script>", version: "1.2", ...(local ? { packages: [{ identifier: key, transport: { type: "stdio" } }] } : { remotes: [{ type: "streamable-http", url: "https://owned.invalid/mcp" }] }) }, registryMeta: { "io.modelcontextprotocol.registry/official": { status: "active", publishedAt: "2026-09-01T12:00:00Z" } } });
const action = async (work: () => Promise<void>) => { await work(); };
function mount(invoke: any, cache = createDiscoverCache()) { return render(<StrictMode><MCPDiscover cache={cache} invoke={invoke} action={action} busy={false} added={vi.fn()} manual={vi.fn()} /></StrictMode>); }
test("initial directory loads without a query; text and connection filters stay local", async () => {
  const invoke = vi.fn(async (a: any) => ({ page: a.fetch ? { entries: [entry("alpha", "Alpha"), entry("beta", "Beta", true)] } : null }));
  const { container } = mount(invoke);
  await screen.findByRole("button", { name: "Add Alpha" });
  expect(invoke.mock.calls.map(([a]) => a)).toEqual([{ action: "discover", fetch: false }, { action: "discover", query: "", fetch: true }]);
  fireEvent.change(screen.getByLabelText("Search public servers"), { target: { value: "beta" } });
  expect(screen.queryByRole("button", { name: "Add Alpha" })).toBeNull();
  expect(screen.getByRole("button", { name: "Add Beta" })).toBeTruthy();
  fireEvent.change(screen.getByLabelText("Server category"), { target: { value: "remote" } });
  expect(screen.getByText("No matching servers")).toBeTruthy();
  expect(invoke).toHaveBeenCalledTimes(2);
  expect(container.querySelector("script, img")).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "Search registry" }));
  await waitFor(() => expect(invoke).toHaveBeenLastCalledWith({ action: "discover", query: "beta", cursor: "", fetch: true, refresh: false }));
});
test("cached directory is immediate; load more appends distinct rows and preserves them on failure", async () => {
  const first = entry("alpha", "Alpha"), second = entry("beta", "Beta"); let fail = false;
  const invoke = vi.fn(async (a: any) => !a.fetch ? { page: { iconVersion: 1, entries: [first], nextCursor: "next" }, query: "" } : fail ? { page: null, error: "Offline; cached results remain available." } : { page: { iconVersion: 1, entries: [first, second], nextCursor: "last" }, query: "" });
  mount(invoke); await screen.findByRole("button", { name: "Add Alpha" });
  expect(invoke).toHaveBeenCalledTimes(1);
  fireEvent.click(screen.getByRole("button", { name: "Load more" }));
  await screen.findByRole("button", { name: "Add Beta" });
  expect(screen.getAllByRole("button", { name: "Add Alpha" })).toHaveLength(1);
  fail = true; fireEvent.click(screen.getByRole("button", { name: "Refresh source" }));
  await screen.findByText("Offline; cached results remain available.");
  expect(screen.getByRole("button", { name: "Add Beta" })).toBeTruthy();
});
test("row Add inspects native detail without preparing, connecting or inventing capability claims", async () => {
  const e = entry("alpha", "Alpha");
  const invoke = vi.fn(async (a: any) => a.action === "discover" ? { page: { iconVersion: 1, entries: [e] } } : { ...e, sha256: "native-digest", capabilities: "Connect after review to discover actual capabilities.", setup: { remote: [], local: [] } });
  mount(invoke); fireEvent.click(await screen.findByRole("button", { name: "Add Alpha" }));
  const heading = await screen.findByRole("heading", { name: "Alpha" });
  await waitFor(() => expect(document.activeElement).toBe(heading));
  expect(screen.getByText("active · Not verified by AG1")).toBeTruthy();
  expect(screen.getByText("Not specified; checked on connection")).toBeTruthy();
  expect(invoke.mock.calls.map(([a]) => a.action)).toEqual(["discover", "discover_detail"]);
});

test("publisher icons use only native raster data and pass listing keys, never image URLs", async () => {
  const first = { ...entry("alpha", "Alpha"), iconUrl: "https://avatars.githubusercontent.com/u/1" };
  const second = { ...entry("beta", "Beta"), iconUrl: "https://avatars.githubusercontent.com/u/2" };
  const raster = "data:image/png;base64,aGVsbG8=";
  const invoke = vi.fn(async (a: any) => a.action === "discover" ? { page: { iconVersion: 1, entries: [first, second] } } : { dataUrl: a.key === "alpha" ? raster : "https://untrusted.invalid/tracker.png" });
  const { container } = mount(invoke);
  await waitFor(() => expect(container.querySelector("img")?.getAttribute("src")).toBe(raster));
  await waitFor(() => expect(invoke).toHaveBeenCalledWith({ action: "discover_icon", key: "beta" }));
  expect(container.querySelectorAll("img")).toHaveLength(1);
  expect(invoke.mock.calls.every(([a]) => !Object.hasOwn(a, "url"))).toBe(true);
});

test("reopening retains loaded pages and raster icons before native cache replies", async () => {
  const cache = createDiscoverCache();
  const first = { ...entry("warm", "Warm"), iconUrl: "https://avatars.githubusercontent.com/u/99" };
  const second = entry("next", "Next");
  const raster = "data:image/png;base64,aGVsbG8=";
  const invoke = vi.fn(async (a: any) => a.action === "discover_icon" ? { dataUrl: raster } : { query: "", page: { iconVersion: 1, fetchedAt: 123, entries: a.cursor ? [second] : [first], nextCursor: a.cursor ? null : "next" } });
  const mounted = mount(invoke, cache);
  await waitFor(() => expect(mounted.container.querySelector("img")?.getAttribute("src")).toBe(raster));
  fireEvent.click(screen.getByRole("button", { name: "Load more" }));
  await screen.findByRole("button", { name: "Add Next" });
  mounted.unmount();
  const reopened = mount(vi.fn(() => new Promise(() => {})), cache);
  expect(screen.getByRole("button", { name: "Add Warm" })).toBeTruthy();
  expect(screen.getByRole("button", { name: "Add Next" })).toBeTruthy();
  expect(reopened.container.querySelector("img")?.getAttribute("src")).toBe(raster);
});
test("missing icons load concurrently with at most four requests; compact summary filters work", async () => {
  let active = 0, peak = 0; const finish: (() => void)[] = [];
  const rows = Array.from({ length: 6 }, (_, i) => ({ ...entry(`parallel${i}`, `Parallel ${i}`), iconUrl: `https://avatars.githubusercontent.com/u/${i + 100}`, server: { name: `org/parallel${i}`, title: `Parallel ${i}` }, summary: { remote: true, local: false, credentials: true, transport: "streamable-http" } }));
  const invoke = vi.fn((a: any) => a.action === "discover" ? Promise.resolve({ page: { iconVersion: 1, entries: rows } }) : new Promise(resolve => {
    active++; peak = Math.max(peak, active); finish.push(() => { active--; resolve({ dataUrl: "data:image/png;base64,aGVsbG8=" }); });
  }));
  mount(invoke);
  await waitFor(() => expect(finish).toHaveLength(4));
  expect(peak).toBe(4);
  expect(screen.getAllByText("Credentials required")).toHaveLength(6);
  fireEvent.change(screen.getByLabelText("Server category"), { target: { value: "local" } });
  expect(screen.getByText("No matching servers")).toBeTruthy();
  finish.splice(0).forEach(done => done());
  await waitFor(() => expect(finish).toHaveLength(2));
  finish.splice(0).forEach(done => done());
  await waitFor(() => expect(active).toBe(0));
  expect(peak).toBe(4);
});
