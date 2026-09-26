import { afterEach, expect, test, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { MCPResult } from "./MCPResult";
import { JournalImages } from "./ImageAttachment";
afterEach(cleanup);
test("MCP cards keep resource URLs inert and read original bytes only on inspection", async () => {
  const text = '{"result":"<script>untrusted</script>"}';
  const run = vi.fn(async () => ({ base64: btoa(text), next: null }));
  render(<MCPResult run={run} value={{ server:"owned-server", capability:"lookup", outcome:"complete", complete:true, references:[{name:"<img src=x>",uri:"file:///private/no-grant",retained:false}], original:{id:"owned-artifact",kind:"mcp-original-response",bytes:text.length,sha256:"a".repeat(64)} }} />);
  expect(run).not.toHaveBeenCalled();
  expect(screen.getByText("file:///private/no-grant").tagName).toBe("CODE");
  expect(document.querySelector("a, img, script")).toBeNull();
  fireEvent.click(screen.getByRole("button",{name:"Inspect original MCP response"}));
  await waitFor(() => expect(run).toHaveBeenCalledWith({kind:"artifact_read",artifact:"owned-artifact",offset:"0",limit:32768}));
  expect(await screen.findByText(text)).toBeTruthy();
  expect(document.querySelector("script")).toBeNull();
});
test("MCP images identify the originating capability and label raster previews", async () => {
  URL.createObjectURL = vi.fn(() => "blob:owned-preview"); URL.revokeObjectURL = vi.fn();
  HTMLDialogElement.prototype.showModal = function(){this.setAttribute("open","")};
  HTMLDialogElement.prototype.close = function(){this.removeAttribute("open")};
  const text=JSON.stringify({message:{details:{mcp:{source:{originalName:"chart"}}},content:[{type:"image",mimeType:"image/png",data:"eA=="}]}});
  const run=vi.fn(async()=>({base64:btoa(text),next:null}));
  const view=render(<JournalImages reference="owned-result" run={run} />);
  fireEvent.click(await screen.findByRole("button",{name:"Preview MCP chart preview 1"}));
  expect(screen.getByText(/Raster preview · original retained separately/)).toBeTruthy();
  view.unmount(); expect(URL.revokeObjectURL).toHaveBeenCalledWith("blob:owned-preview");
});
