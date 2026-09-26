import { afterEach, expect, test } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import { ActivityStatus, describeActivity } from "./ActivityStatus";
import { WORKING_VERBS, pickWorkingVerb } from "./workingVerbs";
import type { ClientMessage } from "./generated/protocol";
afterEach(cleanup);
const call = (name: string, args: Record<string, string> = {}): ClientMessage => ({ id: "a", role: "assistant", blocks: [{ kind: "tool", call_id: "c", name, arguments: args }] });

test("varied vocabulary remains stable for the same operation and independent of frames", () => {
  expect(Object.values(WORKING_VERBS).flat().length).toBeGreaterThan(240);
  for (const kind of Object.keys(WORKING_VERBS) as Array<keyof typeof WORKING_VERBS>) {
    expect(new Set(WORKING_VERBS[kind]).size).toBe(WORKING_VERBS[kind].length);
    const samples = Array.from({ length: 100 }, (_, i) => pickWorkingVerb(kind, `turn-${i}`));
    expect(new Set(samples).size).toBeGreaterThan(10);
    expect(pickWorkingVerb(kind, "same")).toBe(pickWorkingVerb(kind, "same"));
  }
  const activity = { kind: "thinking" as const, label: "Thinking", key: "generation-1" };
  const view = render(<ActivityStatus activity={activity} seed="turn-1" />);
  const status = screen.getByRole("status", { name: "Thinking" }), text = status.textContent;
  for (let i = 0; i < 5; i++) view.rerender(<ActivityStatus activity={{ ...activity }} seed="turn-1" />);
  expect(status.textContent).toBe(text);
  expect(status.querySelectorAll(".ag1-activity-mark path")).toHaveLength(3);
  expect(status.querySelector("svg")?.getAttribute("aria-hidden")).toBe("true");
});

test("literal operational states never become a whimsical claim", () => {
  for (const label of ["Waiting for model", "Queued · waiting for model", "Loading Qwen model", "Verifying Qwen files", "Waiting for you", "Stopping response…", "Connection failed"]) {
    const activity = describeActivity(label, [call("edit")]);
    const view = render(<ActivityStatus activity={activity} seed="turn" />);
    expect(screen.getByRole("status", { name: label }).textContent).toBe(label);
    expect(view.container.querySelector(".ag1-activity-word")).toBeNull();
    if (/Queued|Waiting for you|failed/.test(label)) expect(view.container.querySelector("[data-paused]")).not.toBeNull();
    view.unmount();
  }
});

test("only unresolved tool activity supplies a tool verb, never completed searches or prompt guesses", () => {
  expect(describeActivity("Running tools", [call("webSearch")]).kind).toBe("searching");
  expect(describeActivity("Running tools", [call("webFetch")]).kind).toBe("reading");
  expect(describeActivity("Running tools", [call("inspect", { query: "foo" })]).kind).toBe("searching");
  expect(describeActivity("Running tools", [call("edit")]).kind).toBe("editing");
  expect(describeActivity("Generating operation", [call("edit")]).kind).toBe("writing");
  const done: ClientMessage = { id: "r", role: "toolResult", tool: { name: "webSearch", call_id: "c", failed: false }, blocks: [] };
  expect(describeActivity("Running tools", [call("webSearch"), done]).kind).toBe("executing");
  expect(describeActivity("Waiting for model", [call("webSearch"), done]).kind).toBeUndefined();
  expect(describeActivity("Thinking", [call("webSearch"), done]).kind).toBe("thinking");
});

test("build and test labels use actual command positions, with conservative fallback", () => {
  for (const command of ["cargo test --lib", "npm run test", "python verify.py", "pytest -q", "CARGO_BUILD_JOBS=6 cargo test --lib"]) expect(describeActivity("Running tools", [call("bash", { command })]).kind).toBe("testing");
  for (const command of ["cargo build --release", "npm run build", "cmake --build build"]) expect(describeActivity("Running tools", [call("bash", { command })]).kind).toBe("building");
  for (const command of ["echo cargo test", "cat test.py", "git status", "python application.py", "echo 'npm run build'", "cd a && something-unclassified"]) expect(describeActivity("Running tools", [call("bash", { command })]).kind).toBe("executing");
});

test("streamed text updates preserve the chosen verb and stopping takes precedence", () => {
  const live = { id: "g", committed: false, blocks: [{ kind: "thinking" as const, text: "A" }] };
  const first = describeActivity("Thinking", [], live);
  expect(describeActivity("Thinking", [], { ...live, blocks: [{ kind: "thinking", text: "More thought tokens" }] })).toEqual(first);
  const text = { ...live, blocks: [{ kind: "text" as const, text: "Answer text" }] };
  expect(describeActivity("Thinking", [], text).kind).toBe("writing");
  expect(describeActivity("Stopping response…", [], text).label).toBe("Stopping response…");
  expect(describeActivity("Stopping response…", [], text).kind).toBeUndefined();
});
