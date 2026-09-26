import { afterEach, expect, test, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { GitHubResult } from "./GitHubResult";
const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
afterEach(() => { cleanup(); vi.clearAllMocks(); });
const call: any = { kind: "tool", name: "github", call_id: "native-call", arguments: { operation: "issues.create", repo: "owned/fixture" } };
const message = (value: any, failed = false): any => ({ id: "native-result", role: "toolResult", blocks: [{ kind: "text", text: JSON.stringify(value) }], tool: { name: "github", call_id: "native-call", failed } });
test("native GitHub outcomes are readable without opening the activity tree", () => {
  const expand = vi.fn();
  render(<GitHubResult call={call} message={message({ outcome: "applied", status: 201, data: { number: 8, title: "Owned issue", html_url: "https://github.com/owned/fixture/issues/8" } })} expand={expand}><p>Technical output</p></GitHubResult>);
  expect(screen.getByText("Issue created · #8")).toBeTruthy(); expect(screen.queryByText("Technical output")).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "Inspect GitHub result" })); expect(expand).toHaveBeenCalledOnce(); expect(screen.getByText("Technical output")).toBeTruthy();
});
test("uncertain or failed writes cannot display a successful action or auto-retry", () => {
  render(<GitHubResult call={call} message={message({ outcome: "unknown", message: "Interrupted." }, true)} expand={vi.fn()} />);
  expect(screen.getByText("GitHub outcome uncertain")).toBeTruthy(); expect(screen.queryByText(/Issue created/)).toBeNull(); expect(invoke).not.toHaveBeenCalled();
});
test("result links and literal titles cannot open foreign origins or active markup", () => {
  render(<GitHubResult call={call} message={message({ outcome: "applied", status: 201, data: { title: "<script>literal</script>", html_url: "https://github.com.evil.test/steal" } })} expand={vi.fn()} />);
  expect(screen.getByText("<script>literal</script>")).toBeTruthy(); expect(document.querySelector("script")).toBeNull(); expect(screen.queryByRole("button", { name: "Open on GitHub" })).toBeNull();
});

test("local checkout results name the branch and retained recovery without inventing a remote action", () => {
  render(<GitHubResult call={{ ...call, arguments: { operation: "git.checkout", repo: "owned/fixture" } }} message={message({ outcome: "applied", operation: "checkout", branch: "refs/heads/reviewed", commit: "a".repeat(40), recovery_path: "/owned/.ag1-git-recovery-native" })} expand={vi.fn()} />);
  expect(screen.getByText("Branch checked out")).toBeTruthy();expect(screen.getByText(/Recovery files: \/owned/)).toBeTruthy();
  expect(screen.queryByRole("button", { name: "Open on GitHub" })).toBeNull();
});
