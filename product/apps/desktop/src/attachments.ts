import type { Command, JsonValue, PromptInput } from "./generated/protocol";
export async function uploadMessage(text: string, files: File[], run: (command: Command) => Promise<JsonValue>): Promise<PromptInput> {
  if (!files.length) return { kind: "inline", text };
  if (files.length > 8 || files.reduce((sum, file) => sum + file.size, 0) > 2 * 1024 * 1024) throw new Error("Attachments exceed the 2 MiB message limit");
  const content: JsonValue[] = [{ type: "text", text }];
  for (const file of files) {
    const bytes = new Uint8Array(await file.arrayBuffer());
    if (file.type.startsWith("image/")) { let binary = ""; for (const byte of bytes) binary += String.fromCharCode(byte); content.push({ type: "image", mimeType: file.type, data: btoa(binary) }); }
    else { content.push({ type: "text", text: `Attachment: ${file.name}\n${new TextDecoder("utf-8", { fatal: true }).decode(bytes)}` }); }
  }
  const bytes = new TextEncoder().encode(JSON.stringify({ role: "user", content }));
  const sha = Array.from(new Uint8Array(await crypto.subtle.digest("SHA-256", bytes)), byte => byte.toString(16).padStart(2, "0")).join("");
  const initial = await run({ kind: "artifacts", action: "upload_begin", args: {} }) as { upload: string };
  try {
    for (let offset = 0; offset < bytes.length; offset += 32768) { let binary = ""; for (const byte of bytes.subarray(offset, offset + 32768)) binary += String.fromCharCode(byte); await run({ kind: "artifacts", action: "upload_append", args: { upload: initial.upload, offset: String(offset), base64: btoa(binary) } }); }
    const result = await run({ kind: "artifacts", action: "upload_finish", args: { upload: initial.upload, sha256: sha } }) as { artifact: string };
    return { kind: "message_artifact", artifact: result.artifact };
  } catch (error) { await run({ kind: "artifacts", action: "upload_cancel", args: { upload: initial.upload } }).catch(() => {}); throw error; }
}
