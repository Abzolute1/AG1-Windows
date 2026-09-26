import { afterEach, expect, test, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { MCPManager } from "./MCPManager";
afterEach(() => { cleanup(); vi.clearAllMocks(); });
const record = { id: "owned-server", revision: 2, draft: { name: "Owned MCP", protocol: "2026-07-28", auth_profile: null, provenance: { source: "manual" }, configuration: { transport: "http", url: "https://owned.invalid/mcp", loopback_development: false } }, availability: { enabled: true, all_workspaces: false, general_chat: true } };
const listing = () => ({ servers: [{ record, availableHere: true }], connections: [], scope: { kind: "generalChat" } });
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
