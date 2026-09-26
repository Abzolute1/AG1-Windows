import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import App from "./App";
import { ApprovalReview, approvalExpired } from "./ApprovalReview";

vi.mock("@tauri-apps/api/event", () => ({ listen: async () => () => {} }));
const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date("2026-09-09T12:00:00Z"));
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
  vi.stubGlobal("matchMedia", () => ({ matches: false }));
});
afterEach(() => { cleanup(); vi.useRealTimers(); vi.unstubAllGlobals(); vi.clearAllMocks(); });

function fixture() {
  const binding = { id: "request", operation: "operation", argument_sha256: "args",
    scope: { workspace: "ws", session: "s", instance: "i", epoch: "1" }, revision: "1",
    expires_ms: String(Date.now() + 2000) };
  const snapshot = { version: { major: 1, minor: 0, schema: "fixture" }, scope: binding.scope,
    revision: "1", watermark: "1", workspace_path: "/fixture", status: "awaiting_input",
    active_operation: "operation", messages: [], message_count: 0,
    pending: [{ binding, title: "Review this write", kind: "approval", options: [], evidence: "request", truncated: false }],
    model: "fixture", diagnostic: null, session_file: null, fixture_provider: true };
  invoke.mockImplementation(async (name: string) => name === "catalog" ? { projects: [], theme: "light" }
    : name === "bridge_view" ? { snapshot: structuredClone(snapshot), error: null, progress: "" }
    : name === "bridge_command" ? { kind: "reply", status: "accepted", data: null } : null);
  return snapshot;
}
const allow = () => screen.getByRole("button", { name: "Allow this operation" }) as HTMLButtonElement;
const approvals = () => invoke.mock.calls.filter(([name, args]) => name === "bridge_command" && args.envelope.command.kind === "approve");
async function advance(ms: number) { await act(async () => { await vi.advanceTimersByTimeAsync(ms); }); }

test("an unchanged reconnected pending snapshot disables Allow at expiry", async () => {
  fixture(); render(<App />);
  await advance(500);
  expect(allow().disabled).toBe(false);
  await advance(1499);
  expect(allow().disabled).toBe(false);
  await advance(1);
  expect(screen.getByText(/This request expired/)).toBeTruthy();
  expect(allow().disabled).toBe(true);
  fireEvent.click(allow());
  expect(approvals()).toHaveLength(0);
});

test("a click past the deadline cannot submit while timer callbacks are delayed", async () => {
  const snapshot = fixture(); render(<App />);
  await advance(500);
  expect(allow().disabled).toBe(false);
  // Simulate a background clock jump without delivering any timer callback.
  vi.setSystemTime(Number(snapshot.pending[0].binding.expires_ms));
  await act(async () => { fireEvent.click(allow()); });
  expect(approvals()).toHaveLength(0);
  expect(allow().disabled).toBe(true);
  expect(screen.getByText(/No approval can be applied/)).toBeTruthy();
  fireEvent.focus(window);
  expect(allow().disabled).toBe(true);
  expect(screen.getByText(/No approval can be applied/)).toBeTruthy();
});

test("a replacement request gets a fresh deadline and still requires full review", async () => {
  const snapshot = fixture(); render(<App />);
  await advance(2000);
  expect(allow().disabled).toBe(true);
  snapshot.pending = [{ ...snapshot.pending[0], title: "Replacement write", truncated: true,
    binding: { ...snapshot.pending[0].binding, id: "replacement", expires_ms: String(Date.now() + 5000) } }];
  await advance(500);
  expect(screen.getByText("Replacement write")).toBeTruthy();
  expect(screen.queryByText(/This request expired/)).toBeNull();
  expect(allow().disabled).toBe(true);
  expect(screen.getByRole("button", { name: "Read complete request" })).toBeTruthy();
  expect(approvals()).toHaveLength(0);
});

test("a timely approval retains its exact binding and Enter remains blocked", async () => {
  const snapshot = fixture(); render(<App />);
  await advance(500);
  expect(fireEvent.keyDown(allow(), { key: "Enter" })).toBe(false);
  expect(approvals()).toHaveLength(0);
  await act(async () => { fireEvent.click(allow()); });
  expect(approvals()).toHaveLength(1);
  expect(approvals()[0][1].envelope.command).toEqual({ kind: "approve", binding: snapshot.pending[0].binding, allow: true });
});

test.each(["Deny", "Close Review tool request"])("%s still denies the exact pending request", async name => {
  const snapshot = fixture(); render(<App />);
  await advance(500);
  await act(async () => { fireEvent.click(screen.getByRole("button", { name })); });
  expect(approvals()).toHaveLength(1);
  expect(approvals()[0][1].envelope.command).toEqual({ kind: "approve", binding: snapshot.pending[0].binding, allow: false });
});

test("expiry timers stop at the deadline, restart for changed deadlines, and clean up on unmount", async () => {
  const review = (expires: string) => <ApprovalReview title="Review" expires={expires}>{expired => <button disabled={expired}>Allow</button>}</ApprovalReview>;
  const component = render(review(String(Date.now() + 1250)));
  expect(vi.getTimerCount()).toBe(1);
  await advance(1250);
  expect((screen.getByRole("button") as HTMLButtonElement).disabled).toBe(true);
  expect(vi.getTimerCount()).toBe(0);
  component.rerender(review(String(Date.now() + 5000)));
  expect((screen.getByRole("button") as HTMLButtonElement).disabled).toBe(false);
  expect(vi.getTimerCount()).toBe(1);
  vi.setSystemTime(Date.now() + 5000);
  fireEvent(document, new Event("visibilitychange"));
  expect((screen.getByRole("button") as HTMLButtonElement).disabled).toBe(true);
  component.rerender(review(String(Date.now() + 5000)));
  component.unmount();
  fireEvent.focus(window);
  fireEvent(document, new Event("visibilitychange"));
  expect(vi.getTimerCount()).toBe(0);
});

test.each([undefined, "", "invalid", "Infinity"])("invalid deadline %s cannot enable approval", expires => {
  expect(approvalExpired(expires)).toBe(true);
  render(<ApprovalReview title="Review" expires={expires}>{expired => <button disabled={expired}>Allow</button>}</ApprovalReview>);
  expect((screen.getByRole("button") as HTMLButtonElement).disabled).toBe(true);
  expect(vi.getTimerCount()).toBe(0);
});

test("compact review keeps command, target and proposed change outside collapsed help", () => {
  const details = "printf '<script>literal only</script>'\n\nin /fixture/project";
  const changes = "+ <script>literal only</script>";
  const { container } = render(<ApprovalReview title={JSON.stringify({ summary: "Run this command", details, changes })} expires={String(Date.now() + 5000)} />);
  expect(screen.getByText("Run this command")).toBeTruthy();
  expect(screen.getByLabelText("Request details").textContent).toBe(details);
  expect(screen.getByLabelText("Proposed change").textContent).toBe(changes);
  expect(screen.getByLabelText("Request details").closest("details")).toBeNull();
  expect(screen.getByLabelText("Proposed change").closest("details")).toBeNull();
  expect(container.querySelector("script")).toBeNull();
  expect(container.querySelector("details")?.open).toBe(false);
  expect(screen.getByRole("timer").getAttribute("aria-live")).toBe("off");
  expect(screen.getByText("Closing denies this request.")).toBeTruthy();
});

test("native Git approvals share readable targets and recovery choices without hiding exact data", () => {
  const preview={action:"recover",folder:"/owned/project",repository:"owned/repo",target_branch:"refs/heads/main",target_sha:"a".repeat(40),choice:"restore",recovery_warning:"Later edits are refused.",files:[{path:"literal <script>.txt",operation:"restore retained bytes"}]};
  const {container}=render(<ApprovalReview title={JSON.stringify({summary:"Review local Git change",kind:"native_local_git",details:JSON.stringify(preview)})} expires={String(Date.now()+5000)} />);
  const summary=screen.getByLabelText("Reviewed Git change");
  expect(summary.textContent).toContain("/owned/project");expect(summary.textContent).toContain("refs/heads/main");expect(screen.getByText("Recovery choice: restore recorded starting state").closest("details")).toBeNull();
  expect(screen.getByLabelText("Exact local Git preview").closest("details")?.open).toBe(false);
  expect(JSON.parse(screen.getByLabelText("Exact local Git preview").textContent!)).toEqual(preview);
  expect(container.querySelector("script")).toBeNull();
});

test("malformed Git presentation falls back to literal request details", () => {
  const details=JSON.stringify({action:"stage",folder:"/owned",repository:"owned/repo",files:[{path:"x",operation:{unexpected:"object"}}]});
  render(<ApprovalReview title={JSON.stringify({summary:"Review local Git change",kind:"native_local_git",details})} expires={String(Date.now()+5000)} />);
  expect(screen.queryByLabelText("Reviewed Git change")).toBeNull();expect(screen.getByLabelText("Request details").textContent).toBe(details);
});

test("critical target warning is visible and a new request resets expanded help", () => {
  const review = (details: string) => <ApprovalReview title={JSON.stringify({ summary: "Edit a file outside this project", risk: "critical", details })} expires={String(Date.now() + 5000)} />;
  const { container, rerender } = render(review("File: /outside/first.txt"));
  expect(screen.getByText(/Outside the current project/).closest("details")).toBeNull();
  const help = container.querySelector("details")!;
  help.open = true;
  rerender(review("File: /outside/second.txt"));
  expect(container.querySelector("details")?.open).toBe(false);
  expect(screen.getByLabelText("Request details").textContent).toBe("File: /outside/second.txt");
});

test.each(["{malformed", JSON.stringify({ summary: 42, details: "Keep this literal" }), "x".repeat(32769)])("unrecognized review data remains literal (%#)", title => {
  const { container } = render(<ApprovalReview title={title} />);
  expect(container.querySelector(".approval-summary")?.textContent).toBe(title);
  expect(screen.queryByLabelText("Request details")).toBeNull();
});

test("decision buttons and expiry are outside the scrolling request", async () => {
  fixture(); render(<App />);
  await advance(500);
  expect(screen.getByRole("dialog").classList.contains("approval-dialog")).toBe(true);
  expect(allow().closest(".approval-body")).toBeNull();
  expect(allow().closest(".approval-footer")).toBeTruthy();
  expect(screen.getByRole("timer").closest(".approval-footer")).toBeTruthy();
});
