import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { DraftAttachment, JournalImages } from "./ImageAttachment";
import { uploadMessage } from "./attachments";
import { webcrypto } from "node:crypto";
import { TurnView } from "./NativeConversation";
const png = "iVBORw0KGgoAAAANSUhEUgAAAAgAAAAICAIAAABLbSncAAAAEElEQVR4nGNgaGjAjoaWBADiMEABWMSWggAAAABJRU5ErkJggg==";
beforeEach(() => {
  let urlIndex = 0; URL.createObjectURL = vi.fn(() => urlIndex++ === 0 ? "blob:image-fixture" : `blob:image-fixture-${urlIndex}`); URL.revokeObjectURL = vi.fn();
  HTMLDialogElement.prototype.showModal = function () { this.setAttribute("open", ""); };
  HTMLDialogElement.prototype.close = function () { this.removeAttribute("open"); };
});
afterEach(() => { cleanup(); vi.unstubAllGlobals(); });
test("draft image previews its original pixels and removal releases its URL", async () => {
  const file = new File([Uint8Array.from(atob(png), c => c.charCodeAt(0))], "clipboard.png", { type: "image/png" });
  const remove = vi.fn(), view = render(<DraftAttachment file={file} remove={remove} />);
  fireEvent.click(await screen.findByRole("button", { name: "Preview clipboard.png" }));
  expect(screen.getByRole("dialog", { name: "Preview clipboard.png" })).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "View at full resolution" }));
  expect(document.querySelector(".original-size img")?.getAttribute("src")).toBe("blob:image-fixture");
  fireEvent.click(screen.getByRole("button", { name: "Close image preview" }));
  expect(screen.queryByRole("dialog")).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "Remove clipboard.png" })); expect(remove).toHaveBeenCalledOnce();
  view.unmount(); expect(URL.revokeObjectURL).toHaveBeenCalledWith("blob:image-fixture");
});
test("sent images hydrate from paged journal bytes, survive polling, and open full previews", async () => {
  const content = JSON.stringify({ message: { content: [{ type: "image", mimeType: "image/png", data: png }, { type: "text", text: "Two images" }, { type: "image", mimeType: "image/png", data: png }] } });
  const run = vi.fn(async (c: any) => c.args.offset === "0" ? { base64: btoa(content.slice(0, 30)), next: "30" } : { base64: btoa(content.slice(30)), next: null });
  const view = render(<JournalImages reference="user-message" run={run} />);
  await screen.findByRole("button", { name: "Preview Image 2" });
  view.rerender(<JournalImages reference="user-message" run={run} />); expect(run).toHaveBeenCalledTimes(2);
  fireEvent.click(screen.getByRole("button", { name: "Preview Image 1" }));
  expect(screen.getByRole("dialog")).toBeTruthy(); view.unmount(); expect(URL.revokeObjectURL).toHaveBeenCalledTimes(2);
});
test("image-only upload preserves exact original bytes and MIME in the native message artifact", async () => {
  vi.stubGlobal("crypto", webcrypto);
  const bytes = Uint8Array.from(atob(png), c => c.charCodeAt(0));
  const file = new File([bytes], "image.png", { type: "image/png" });
  Object.defineProperty(file, "arrayBuffer", { value: async () => bytes.buffer });
  let encoded = "";
  const run = vi.fn(async (c: any): Promise<import("./generated/protocol").JsonValue> => { if (c.action === "upload_begin") return { upload: "u" }; if (c.action === "upload_append") encoded += atob(c.args.base64); return { artifact: "image-artifact" }; });
  expect(await uploadMessage("", [file], run)).toEqual({ kind: "message_artifact", artifact: "image-artifact" });
  expect(JSON.parse(encoded).content[1]).toEqual({ type: "image", mimeType: "image/png", data: png });
});

test("tool images appear inline with collapsed diagnostics and are not duplicated on expansion", async () => {
  const message = { message: { content: [{ type: "text", text: "Read image file [image/png]" }, { type: "image", mimeType: "image/png", data: png }] } };
  const run = vi.fn(async () => ({ base64: btoa(JSON.stringify(message)), next: null }));
  const turn = { id: "u", messages: [
    { id: "u", role: "user", blocks: [{ kind: "text", text: "Show the image" }] },
    { id: "a", role: "assistant", blocks: [{ kind: "tool", call_id: "c", name: "read", arguments: { path: "ball.png" } }] },
    { id: "r", role: "toolResult", tool: { name: "read", call_id: "c", failed: false }, blocks: [{ kind: "text", text: "Read image file [image/png]" }, { kind: "attachment", media_type: "image/png", reference: "r" }, { kind: "attachment", media_type: "image/png", reference: "r" }] },
    { id: "f", role: "assistant", blocks: [{ kind: "text", text: "Here is the image." }] },
  ] } as any;
  const view = render(<TurnView turn={turn} active={false} run={run} />);
  const button = await screen.findByRole("button", { name: "Preview Image 1" });
  expect(button.closest(".image-results")).not.toBeNull();
  expect(button.closest(".work-summary")).toBeNull();
  expect(button.closest(".inline-image")).not.toBeNull();
  expect(screen.queryByRole("region", { name: "Tool output" })).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "Work details 1 tool", expanded: false }));
  fireEvent.click(screen.getByRole("button", { name: /Read ball.png/ }));
  expect(screen.getByRole("region", { name: "Tool output" }).textContent).toContain("Read image file");
  expect(screen.getAllByRole("button", { name: "Preview Image 1" })).toHaveLength(1);
  view.rerender(<TurnView turn={{ ...turn }} active={false} run={run} />);
  expect(run).toHaveBeenCalledOnce();
});

test("SVG preview offers original source as download-only data, never rendered SVG markup", async () => {
  const source = '<svg xmlns="http://www.w3.org/2000/svg"><circle r="4"/></svg>';
  const message = { message: { content: [{ type: "image", mimeType: "image/png", data: png }], details: { imagePreview: { name: "ball.svg", sourceMimeType: "image/svg+xml", sourceBase64: btoa(source) } } } };
  const run = vi.fn(async () => ({ base64: btoa(JSON.stringify(message)), next: null }));
  const view = render(<JournalImages reference="svg-result" run={run} inline />);
  await screen.findByRole("button", { name: "Preview ball.svg preview" });
  const link = screen.getByRole("link", { name: "Download ball.svg" });
  expect(link.getAttribute("download")).toBe("ball.svg");
  const blob = vi.mocked(URL.createObjectURL).mock.calls[1][0] as Blob;
  expect(blob.type).toBe("application/octet-stream");
  const decoded = await new Promise<string>(resolve => { const reader = new FileReader(); reader.onload = () => resolve(String(reader.result)); reader.readAsText(blob); });
  expect(decoded).toBe(source);
  expect(document.querySelector("circle")).toBeNull();
  expect(document.querySelector("iframe")).toBeNull();
  view.unmount(); expect(URL.revokeObjectURL).toHaveBeenCalledTimes(2);
});

test("a failed tool image is not promoted as a successful result", () => {
  const run = vi.fn(async () => null);
  render(<TurnView active={false} run={run} turn={{ id: "u", messages: [{ id: "r", role: "toolResult", tool: { name: "read", call_id: "c", failed: true }, blocks: [{ kind: "attachment", media_type: "image/png", reference: "r" }] }] }} />);
  expect(document.querySelector(".image-results")).toBeNull(); expect(run).not.toHaveBeenCalled();
});

test("image journal errors stay visible and can be retried", async () => {
  const run = vi.fn().mockRejectedValueOnce(new Error("Unavailable")).mockResolvedValue({ base64: btoa(JSON.stringify({ message: { content: [{ type: "image", mimeType: "image/png", data: png }] } })), next: null });
  render(<JournalImages reference="retry-image" run={run} inline />);
  fireEvent.click(await screen.findByRole("button", { name: "Retry image" }));
  expect(await screen.findByRole("button", { name: "Preview Image 1" })).toBeTruthy();
  expect(screen.queryByRole("alert")).toBeNull();
});
