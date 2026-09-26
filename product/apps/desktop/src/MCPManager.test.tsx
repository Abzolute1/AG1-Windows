import { afterEach, expect, test, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { MCPManager } from "./MCPManager";
afterEach(() => { cleanup(); vi.clearAllMocks(); });
const record = { id: "owned-server", revision: 2, draft: { name: "Owned MCP", protocol: "2026-07-28", auth_profile: null, provenance: { source: "manual" }, configuration: { transport: "http", url: "https://owned.invalid/mcp", loopback_development: false } }, availability: { enabled: true, all_workspaces: false, general_chat: true } };
const listing = () => ({ servers: [{ record, availableHere: true }], connections: [], scope: { kind: "generalChat" } });

test("permission rules require native review and confirmation, then revoke the saved native wire identity", async () => {
  let saved = false, revoked = false;
  const rules = [{ arguments: { kind: "equals", fields: { value: 1 } }, effect: "write", permission: "allow" }];
  const detail = { identity: { originalName: "owned", kind: "tool" }, original: { annotations: { readOnlyHint: true } }, parameters: { properties: { value: { type: "integer" } } }, schemaSupported: true, classification: "unknown", classificationBasis: "Native target bound", wireName: "mcp_native_wire" };
  const run = vi.fn(async (args: any) => {
    if (args.action === "catalog") return { items: [{ key: "owned", name: "Owned capability" }], total: 1, binding: { protocol: "2026-07-28" } };
    if (args.action === "detail") return detail;
    if (args.action === "trust_review") return { review: "native-rule-review", details: { serverName: "Owned MCP", configuration: record.draft.configuration, scope: { kind: "generalChat" }, identity: detail.identity, targetDigest: "native-digest", rules: args.rules, basis: "Review this exact native target." } };
    if (args.action === "trust_confirm") { saved = true; return { message: "Reviewed rules saved. Reconnect to apply." }; }
    if (args.action === "trust_revoke") { revoked = true; return { message: "Rules revoked." }; }
    return { ...listing(), servers: [{ record: { ...record, revision: saved ? 3 : 2 }, availableHere: true, trustPolicy: { mode: "restricted", grants: saved && !revoked ? [{ identity: detail.identity, wireName: "mcp_native_wire", rules }] : [] } }], connections: saved ? [] : [{ id: "owned-connection", serverId: record.id, health: "ready" }] };
  });
  render(<MCPManager scope="owned" run={run} back={vi.fn()} />);
  fireEvent.click(await screen.findByRole("button", { name: /Owned MCP/ }));
  fireEvent.click(screen.getByRole("button", { name: "Tools" }));
  const capability = await screen.findByRole("button", { name: "Owned capability" });
  await waitFor(() => expect(capability.hasAttribute("disabled")).toBe(false)); fireEvent.click(capability);
  const effect = await screen.findByLabelText("Reviewed effect");
  await waitFor(() => expect(screen.queryByText("Updating MCP…")).toBeNull());
  fireEvent.change(effect, { target: { value: "write" } });
  fireEvent.change(screen.getByLabelText("Approval behavior"), { target: { value: "allow" } });
  fireEvent.change(screen.getByLabelText("Argument name 1"), { target: { value: "value" } });
  fireEvent.change(screen.getByLabelText("Value type 1"), { target: { value: "number" } });
  fireEvent.change(screen.getByLabelText("Exact value 1"), { target: { value: "1" } });
  fireEvent.click(screen.getByRole("button", { name: "Review rule" }));
  const confirm = await screen.findByRole("button", { name: "Save reviewed rules" });
  expect(confirm.hasAttribute("disabled")).toBe(true);
  expect(run).toHaveBeenCalledWith({ action: "trust_review", connection: "owned-connection", revision: 2, kind: "tool", key: "owned", rules, append: true });
  expect(run.mock.calls.some(([a]) => a.action === "trust_confirm")).toBe(false);
  await waitFor(() => expect(screen.queryByText("Updating MCP…")).toBeNull());
  fireEvent.click(screen.getByLabelText("I reviewed the target, scope, effects and exact argument conditions.")); fireEvent.click(confirm);
  await screen.findByText("Reviewed rules saved. Reconnect to apply.");
  expect(run).toHaveBeenCalledWith({ action: "trust_confirm", review: "native-rule-review" });
  const revoke = await screen.findByRole("button", { name: "Revoke rules for owned" });
  await waitFor(() => expect(revoke.hasAttribute("disabled")).toBe(false)); fireEvent.click(revoke);
  await screen.findByText("Rules revoked.");
  expect(run).toHaveBeenCalledWith({ action: "trust_revoke", id: record.id, revision: 3, wire_name: "mcp_native_wire" });
  expect(run.mock.calls.every(([a]) => ["list", "catalog", "detail", "trust_review", "trust_confirm", "trust_revoke"].includes(a.action))).toBe(true);
});

test("scope policy changes require save and do not connect or activate a server", async () => {
  const run = vi.fn(async (args: any) => args.action === "trust_mode" ? { message: "Policy saved." } : listing());
  render(<MCPManager scope="owned" run={run} back={vi.fn()} />);
  fireEvent.click(await screen.findByRole("button", { name: /Owned MCP/ }));
  fireEvent.change(screen.getByLabelText("Scope policy"), { target: { value: "disableWrites" } });
  expect(run.mock.calls.every(([a]) => a.action === "list")).toBe(true);
  fireEvent.click(screen.getByRole("button", { name: "Save scope policy" }));
  await screen.findByText("Policy saved.");
  expect(run).toHaveBeenCalledWith({ action: "trust_mode", id: record.id, revision: 2, mode: "disableWrites" });
  expect(run.mock.calls.every(([a]) => ["list", "trust_mode"].includes(a.action))).toBe(true);
});
test("opening Installed only reads native metadata and add saves an inactive draft", async () => {
  const run = vi.fn(async (args: any) => args.action === "list" ? listing() : { ids: ["new-server"], active: false });
  render(<MCPManager scope="owned" run={run} back={vi.fn()} />);
  await screen.findByText("General chat", { exact: false });
  expect(run.mock.calls.every(([args]) => args.action === "list")).toBe(true);
  fireEvent.click(screen.getByRole("button", { name: "Add server" }));
  fireEvent.change(screen.getByLabelText("Display name"), { target: { value: "New endpoint" } });
  fireEvent.change(screen.getByLabelText("MCP URL"), { target: { value: "https://new.invalid/mcp" } });
  fireEvent.click(screen.getByRole("button", { name: "Save inactive" }));
  await waitFor(() => expect(run).toHaveBeenCalledWith({ action: "add", draft: { ...record.draft, name: "New endpoint", configuration: { transport: "http", url: "https://new.invalid/mcp", loopback_development: false } } }));
  expect(run.mock.calls.some(([args]) => ["review", "confirm", "availability"].includes(args.action))).toBe(false);
});
test("connecting requires the exact native review and an explicit confirmation", async () => {
  const run = vi.fn(async (args: any) => args.action === "list" ? listing() : args.action === "review" ? { review: "native-review", server: record, warning: "Reviewed endpoint", purpose: "Discovery only", scope: { kind: "generalChat" } } : { connection: "owned-connection" });
  render(<MCPManager scope="owned" run={run} back={vi.fn()} />);
  fireEvent.click(await screen.findByRole("button", { name: /Owned MCP/ }));
  fireEvent.click(screen.getByRole("button", { name: "Connect" }));
  const confirm = await screen.findByRole("button", { name: "Connect reviewed target" });
  expect(confirm.hasAttribute("disabled")).toBe(true);
  expect(run.mock.calls.some(([args]) => args.action === "confirm")).toBe(false);
  fireEvent.click(screen.getByLabelText("I reviewed the exact target and connection effects."));
  await waitFor(() => expect(confirm.hasAttribute("disabled")).toBe(false));
  fireEvent.click(confirm);
  await waitFor(() => expect(run).toHaveBeenCalledWith({ action: "confirm", review: "native-review" }));
  expect(run.mock.calls.filter(([args]) => args.action === "confirm")).toHaveLength(1);
});
test("local fields retain argument boundaries and reject invalid public environment", async () => {
  const run = vi.fn(async () => listing());
  render(<MCPManager scope="owned" run={run} back={vi.fn()} />);
  await screen.findByRole("button", { name: /Owned MCP/ });
  fireEvent.click(screen.getByRole("button", { name: "Add server" }));
  fireEvent.change(screen.getByLabelText("Display name"), { target: { value: "Local" } });
  fireEvent.change(screen.getByLabelText("Connection type"), { target: { value: "local" } });
  fireEvent.change(screen.getByLabelText("Executable · absolute path"), { target: { value: "/usr/bin/python3" } });
  fireEvent.change(screen.getByLabelText("Working directory · absolute path"), { target: { value: "/owned" } });
  fireEvent.change(screen.getByLabelText("Arguments · JSON array"), { target: { value: '["file with spaces.py","literal $HOME"]' } });
  fireEvent.change(screen.getByLabelText("Public environment · JSON object"), { target: { value: '[]' } });
  fireEvent.click(screen.getByRole("button", { name: "Save inactive" }));
  await screen.findByRole("alert");
  expect(run.mock.calls.some(([args]: any) => args?.action === "add")).toBe(false);
});
test("late scope results cannot replace the new session view", async () => {
  let finish!: (value: any) => void;
  const old = vi.fn(() => new Promise(resolve => { finish = resolve; }));
  const next = vi.fn(async () => ({ servers: [], connections: [], scope: { kind: "workspace" } }));
  const rendered = render(<MCPManager scope="old" run={old} back={vi.fn()} />);
  await waitFor(() => expect(old).toHaveBeenCalled());
  rendered.rerender(<MCPManager scope="new" run={next} back={vi.fn()} />);
  await screen.findByText(/No servers installed/);
  finish(listing());
  await waitFor(() => expect(screen.queryByRole("button", { name: /Owned MCP/ })).toBeNull());
});
test("capability content is literal and inspection never executes a server tool", async () => {
  const run = vi.fn(async (args: any) => args.action === "list" ? { ...listing(), connections: [{ id: "c", serverId: record.id, health: "ready" }] } : args.action === "catalog" ? { items: [{ key: "owned", name: "<script>literal</script>", description: "Untrusted" }], total: 1, next: null, binding: { protocol: "2026-07-28" } } : { original: { inputSchema: { type: "object" } }, classificationBasis: "Unknown risk", approval: "Ask each time" });
  render(<MCPManager scope="owned" run={run} back={vi.fn()} />);
  fireEvent.click(await screen.findByRole("button", { name: /Owned MCP/ }));
  fireEvent.click(screen.getByRole("button", { name: "Tools" }));
  const tool = await screen.findByRole("button", { name: /<script>literal/ });
  expect(tool.querySelector("script")).toBeNull();
  await waitFor(() => expect(screen.queryByRole("status")).toBeNull());
  fireEvent.click(tool);
  await screen.findByText("Capability details");
  expect(run.mock.calls.every(([args]) => ["list", "catalog", "detail"].includes(args.action))).toBe(true);
});
test("private values use native entry with public IDs only and logout remains local", async () => {
  let saved = false, revoked = false;
  const run = vi.fn(async (args: any) => {
    if (args.action === "private_entry") { saved = true; return { saved: true, active: false }; }
    if (args.action === "logout") { revoked = true; return { message: "Local credentials removed. Upstream revocation was not confirmed." }; }
    return { ...listing(), privateEntryAvailable: true, servers: [{ record: { ...record, draft: { ...record.draft, auth_profile: saved ? "native-profile" : null } }, availableHere: true, authentication: saved ? { status: revoked ? "revoked" : "connected", account: null } : null }] };
  });
  render(<MCPManager scope="owned" run={run} back={vi.fn()} />);
  fireEvent.click(await screen.findByRole("button", { name: /Owned MCP/ }));
  fireEvent.click(screen.getByRole("button", { name: "Enter private values" }));
  await screen.findByText("Private credentials saved");
  expect(run).toHaveBeenCalledWith({ action: "private_entry", id: record.id, revision: record.revision });
  expect(document.querySelector('input[type="password"]')).toBeNull();
  expect(run.mock.calls.every(([args]) => ["list", "private_entry"].includes(args.action))).toBe(true);
  fireEvent.click(screen.getByRole("button", { name: "Log out locally" }));
  await screen.findByText("Logged out locally");
  await screen.findByText("Local credentials removed. Upstream revocation was not confirmed.");
  expect(screen.queryByRole("alert")).toBeNull();
});
test("local configuration saves secret names separately from private values", async () => {
  const run = vi.fn(async (args: any) => args.action === "list" ? listing() : { ids: ["local-new"] });
  render(<MCPManager scope="owned" run={run} back={vi.fn()} />);
  await screen.findByRole("button", { name: /Owned MCP/ });
  fireEvent.click(screen.getByRole("button", { name: "Add server" }));
  fireEvent.change(screen.getByLabelText("Display name"), { target: { value: "Private local" } });
  fireEvent.change(screen.getByLabelText("Connection type"), { target: { value: "local" } });
  fireEvent.change(screen.getByLabelText("Executable · absolute path"), { target: { value: "/usr/bin/python3" } });
  fireEvent.change(screen.getByLabelText("Working directory · absolute path"), { target: { value: "/owned" } });
  fireEvent.change(screen.getByLabelText("Private environment names · one per line"), { target: { value: "API_TOKEN\nAPI_PASSWORD" } });
  fireEvent.click(screen.getByRole("button", { name: "Save inactive" }));
  await waitFor(() => expect(run.mock.calls.some(([args]) => args.action === "add")).toBe(true));
  const saved = run.mock.calls.find(([args]) => args.action === "add")![0].draft;
  expect(saved.configuration.secret_environment).toEqual(["API_TOKEN", "API_PASSWORD"]);
  expect(saved.configuration.public_environment).toEqual({});
  expect(run.mock.calls.some(([args]) => ["private_entry", "review", "confirm"].includes(args.action))).toBe(false);
});
test("OAuth buttons send only server identity and revision and never enable or connect", async () => {
  const run = vi.fn(async (args: any) => args.action === "list" ? { ...listing(), oauthAvailable: true } : { saved: true, active: false, message: "Authentication updated. Review and connect separately." });
  render(<MCPManager scope="owned" run={run} back={vi.fn()} />);
  fireEvent.click(await screen.findByRole("button", { name: /Owned MCP/ }));
  fireEvent.click(screen.getByRole("button", { name: "Sign in with OAuth" }));
  await waitFor(() => expect(run).toHaveBeenCalledWith({ action: "oauth", id: record.id, revision: record.revision, refresh: false }));
  expect(run.mock.calls.some(([args]) => ["review", "confirm", "availability", "private_entry"].includes(args.action))).toBe(false);
  expect(screen.queryByLabelText(/token|password|authorization code/i)).toBeNull();
});
test("OAuth state shows unverified account and explicit refresh and local logout", async () => {
  const saved = { ...record, draft: { ...record.draft, auth_profile: "owned-profile" } };
  const run = vi.fn(async (args: any) => args.action === "list" ? { ...listing(), oauthAvailable: true, servers: [{ record: saved, availableHere: true, authentication: { status: "expired", target: { kind: "oauth", issuer: "https://issuer.invalid", resource: "https://owned.invalid" }, account: null, scopes: ["files:read"] } }] } : { message: "Updated" });
  render(<MCPManager scope="owned" run={run} back={vi.fn()} />);
  fireEvent.click(await screen.findByRole("button", { name: /Owned MCP/ }));
  expect(screen.getByText(/Identity unavailable/)).toBeTruthy();
  expect(screen.getByText(/Granted scopes: files:read/)).toBeTruthy();
  expect(screen.getByRole("button", { name: "Sign in again" })).toBeTruthy();
  expect(screen.getByRole("button", { name: "Log out locally" })).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Refresh credentials" }));
  await waitFor(() => expect(run).toHaveBeenCalledWith({ action: "oauth", id: record.id, revision: record.revision, refresh: true }));
});
test("a failed OAuth refresh reconciles native state before enabling another action", async () => {
  let failed = false;
  let finishList!: (value: any) => void;
  const snapshot = (revoked: boolean) => ({ ...listing(), oauthAvailable: true,
    servers: [{ record: { ...record, revision: revoked ? 3 : 2, draft: { ...record.draft, auth_profile: "owned-profile" } }, availableHere: true,
      authentication: { status: revoked ? "revoked" : "connected", target: { kind: "oauth" }, scopes: [] } }],
    connections: revoked ? [] : [{ id: "owned-connection", serverId: record.id, health: "ready" }],
  });
  const run = vi.fn(async (args: any) => {
    if (args.action === "list") return failed ? new Promise(resolve => { finishList = resolve; }) : snapshot(false);
    if (args.action === "oauth") {
      failed = true;
      throw new Error(JSON.stringify({ error: 'Unavailable("MCP OAuth client registration was rejected")' }) + " (command 11111111-2222-3333-4444-555555555555)");
    }
    throw new Error("Unexpected operation");
  });
  render(<MCPManager scope="owned" run={run} back={vi.fn()} />);
  fireEvent.click(await screen.findByRole("button", { name: /Owned MCP/ }));
  fireEvent.click(screen.getByRole("button", { name: "Refresh credentials" }));
  await waitFor(() => expect(finishList).toBeTypeOf("function"));
  expect(screen.getByRole("button", { name: "Sign in again" }).hasAttribute("disabled")).toBe(true);
  expect(screen.getByRole("alert").textContent).toBe("MCP OAuth client registration was rejected");
  finishList(snapshot(true));
  await waitFor(() => expect(screen.queryByRole("button", { name: "Refresh credentials" })).toBeNull());
  expect(screen.queryByText(/ready ·/)).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "Sign in again" }));
  await waitFor(() => expect(run).toHaveBeenLastCalledWith({ action: "list" }));
  expect(run).toHaveBeenCalledWith({ action: "oauth", id: record.id, revision: 3, refresh: false });
  finishList(snapshot(true));
});

test("project import reviews the native snapshot and saves inactive only after acknowledgment", async () => {
  const run = vi.fn(async (args: any) => {
    if (args.action === "import_review") return { review: "native-import", source: "/owned/.mcp.json", sha256: "native-hash", drafts: [{ name: "Imported" }], message: "Review this native snapshot." };
    if (args.action === "import_confirm") return { ids: ["imported"], active: false, message: "Imported inactive." };
    return { ...listing(), projectImportAvailable: true };
  });
  render(<MCPManager scope="owned" run={run} back={vi.fn()} />);
  const open = await screen.findByRole("button", { name: "Import project config" });
  await waitFor(() => expect(open.hasAttribute("disabled")).toBe(false));
  expect(run.mock.calls.every(([a]) => a.action === "list")).toBe(true);
  fireEvent.click(open);
  const save = await screen.findByRole("button", { name: "Save imported servers inactive" });
  expect(save.hasAttribute("disabled")).toBe(true);
  expect(run).toHaveBeenCalledWith({ action: "import_review" });
  await waitFor(() => expect(screen.queryByText("Updating MCP…")).toBeNull());
  fireEvent.click(screen.getByLabelText("I reviewed these project definitions for saving inactive.")); fireEvent.click(save);
  await screen.findByText("Imported inactive.");
  expect(run).toHaveBeenCalledWith({ action: "import_confirm", review: "native-import" });
  expect(run.mock.calls.every(([a]) => ["list", "import_review", "import_confirm"].includes(a.action))).toBe(true);
});

test("Discover loads directory metadata and saves the native-reviewed endpoint without connecting", async () => {
  const entry = { key: "owned-key", server: { name: "Owned public server", description: "Literal <script>metadata</script>", version: "1" }, registryMeta: { "io.modelcontextprotocol.registry/official": { status: "active" } } };
  const page = { iconVersion: 1, entries: [entry], fetchedAt: 1000, excluded: 0 };
  const run = vi.fn(async (args: any) => {
    if (args.action === "discover") return { page: args.fetch ? page : null, sourceName: "GitHub MCP Registry", query: args.query || "" };
    if (args.action === "discover_detail") return { key: entry.key, server: entry.server, sha256: "owned-digest", source: "https://api.mcp.github.com", fetchedAt: 1000, setup: { remote: [{ index: 0, supported: true, target: { url: "https://owned.invalid/mcp" }, guidance: "Review this endpoint." }], local: [] }, capabilities: "Connect to inspect actual capabilities." };
    if (args.action === "discover_review") return { review: "owned-setup", draft: record.draft, message: "Save this exact endpoint inactive." };
    if (args.action === "discover_confirm") return { ids: [record.id], message: "Discovered server saved inactive." };
    return listing();
  });
  render(<MCPManager scope="owned" run={run} back={vi.fn()} />);
  await screen.findByRole("button", { name: /Owned MCP/ }); fireEvent.click(screen.getByRole("button", { name: "Discover" }));
  await waitFor(() => expect(run).toHaveBeenCalledWith({ action: "discover", fetch: false }));
  await waitFor(() => expect(run).toHaveBeenCalledWith({ action: "discover", query: "", fetch: true }));
  fireEvent.change(screen.getByLabelText("Search public servers"), { target: { value: "owned" } });
  const item = await screen.findByRole("button", { name: "Add Owned public server" }); await waitFor(() => expect(item.hasAttribute("disabled")).toBe(false)); fireEvent.click(item);
  const review = await screen.findByRole("button", { name: "Review this remote" }); await waitFor(() => expect(review.hasAttribute("disabled")).toBe(false)); fireEvent.click(review);
  const save = await screen.findByRole("button", { name: "Save discovered server inactive" }); expect(save.hasAttribute("disabled")).toBe(true);
  await waitFor(() => expect(screen.queryByText("Updating MCP…")).toBeNull()); fireEvent.click(screen.getByLabelText("I reviewed this target and source.")); fireEvent.click(save);
  await screen.findByText("Discovered server saved inactive."); expect(run).toHaveBeenCalledWith({ action: "discover_confirm", review: "owned-setup" });
  expect(run.mock.calls.every(([a]) => ["list", "discover", "discover_detail", "discover_review", "discover_confirm"].includes(a.action))).toBe(true);
});
