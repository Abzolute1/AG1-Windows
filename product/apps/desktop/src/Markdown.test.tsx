import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import { Markdown } from "./Markdown";
afterEach(() => { cleanup(); vi.unstubAllGlobals(); });

test("highlighted blocks copy exact source, independently of labels and inline code", async () => {
  const writeText = vi.fn().mockResolvedValue(undefined);
  vi.stubGlobal("navigator", { clipboard: { writeText } });
  const source = 'const greeting = "hello";\n  console.log(greeting);\n';
  const { container } = render(<Markdown text={'Inline `repos.get`.\n\n```javascript\n' + source + '```\n\n```text\nsecond\n```'} />);
  expect(container.querySelector(".hljs-keyword")?.textContent).toBe("const");
  expect(container.querySelector("pre code")?.textContent).toBe(source);
  expect(screen.getAllByRole("button", { name: "Copy code" })).toHaveLength(2);
  fireEvent.click(screen.getAllByRole("button", { name: "Copy code" })[0]);
  await screen.findByRole("button", { name: "Code copied" });
  expect(writeText).toHaveBeenCalledWith(source);
  expect(screen.getAllByRole("button", { name: "Copy code" })).toHaveLength(1);
});

test("unknown languages, explicit text, oversized and unfinished fences remain literal and copyable", () => {
  const code = '<img src=x onerror="alert(1)">';
  const { container, rerender } = render(<Markdown text={'```unknown-language\n' + code + '\n```\n\n```text\nconst x = 1;\n```'} />);
  expect(container.querySelector("img")).toBeNull();
  expect(container.querySelector("pre code")?.textContent).toBe(code + '\n');
  expect(container.querySelector(".hljs-keyword")).toBeNull();
  rerender(<Markdown text={'```js\n' + 'x'.repeat(32769)} />);
  expect(container.querySelector("pre code")?.textContent?.trim()).toHaveLength(32769);
  expect(screen.getByRole("button", { name: "Copy code" })).toBeTruthy();
  rerender(<Markdown text={'```\nparameters: { page: 1, per_page: 10 }\n```'} />);
  expect(container.querySelector(".hljs-number")).toBeTruthy();
});

test("clipboard failures give a manual fallback and do not claim success", async () => {
  vi.stubGlobal("navigator", { clipboard: { writeText: vi.fn().mockRejectedValue(new Error("denied")) } });
  render(<Markdown text={'```text\ncopy me\n```'} />);
  fireEvent.click(screen.getByRole("button", { name: "Copy code" }));
  await waitFor(() => expect(screen.getByRole("status").textContent).toContain("Select the code"));
  expect(screen.queryByRole("button", { name: "Code copied" })).toBeNull();
});
