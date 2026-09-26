import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import { ModelLoadControl } from "./ModelLoadControl";
afterEach(cleanup);
const model = { id: "35B", provider: "llamacpp" };
test("local load is explicit; remote models have no load control", () => {
  const load=vi.fn(); const {rerender}=render(<ModelLoadControl model={model} pending={null} disabled={false} load={load} />);
  expect(load).not.toHaveBeenCalled();fireEvent.click(screen.getByRole("button",{name:"Load"}));expect(load).toHaveBeenCalledOnce();
  rerender(<ModelLoadControl model={{id:"remote",provider:"openai"}} pending={null} disabled={false} load={load} />);
  expect(screen.queryByRole("button")).toBeNull();
});
test("measured percentages, indeterminate loading and ready have distinct states", () => {
  const props={model,pending:null,disabled:false,load:vi.fn()};
  const {rerender}=render(<ModelLoadControl {...props} progress={{model:"35B",phase:"verifying",percent:63,started_at:Date.now()-5000}} />);
  expect(screen.getByRole("progressbar").getAttribute("aria-valuenow")).toBe("63");expect(screen.getByRole("status").textContent).toContain("63%");
  rerender(<ModelLoadControl {...props} progress={{model:"35B",phase:"loading"}} />);
  expect(screen.queryByRole("progressbar")).toBeNull();expect(screen.getByRole("status").textContent).toBe("Loading model");
  rerender(<ModelLoadControl {...props} progress={{model:"35B",phase:"ready"}} />);
  expect(screen.getByRole("status").textContent).toBe("Loaded");expect(screen.queryByRole("button")).toBeNull();
  rerender(<ModelLoadControl {...props} progress={{model:"other",phase:"ready"}} />);
  expect(screen.getByRole("button",{name:"Switch & load"})).toBeTruthy();
});
test("stale ready state cannot hide a queued load, failure can retry, busy blocks swaps", () => {
  const props={model,disabled:false,load:vi.fn()};
  const {rerender}=render(<ModelLoadControl {...props} pending={{model:"35B",started_at:2000}} progress={{model:"35B",phase:"ready",started_at:1000}} />);
  expect(screen.getByRole("status").textContent).toBe("Waiting for model slot");expect(screen.queryByRole("button")).toBeNull();
  rerender(<ModelLoadControl {...props} pending={null} progress={{model:"35B",phase:"failed"}} />);
  expect(screen.getByRole("status").textContent).toBe("Load failed");expect(screen.getByRole("button",{name:"Load"})).toBeTruthy();
  rerender(<ModelLoadControl {...props} pending={null} disabled />);
  expect((screen.getByRole("button") as HTMLButtonElement).disabled).toBe(true);
});
