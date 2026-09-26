import type { Command, JsonValue } from "./generated/protocol";
export type Artifact = { id: string; kind: string; bytes: number; sha256: string; created_at: string; mime_type?: string; name?: string };
export type ArtifactCatalog = { artifacts: Artifact[]; next: JsonValue };
export type ArtifactRun = (command: Command) => Promise<JsonValue>;
export const MAX_ARTIFACT_DOWNLOAD = 8 * 1024 * 1024;
export const isRasterArtifact = (artifact: Artifact) => ["image/png", "image/jpeg", "image/webp", "image/gif"].includes(artifact.mime_type ?? "");
export const artifactSize = (n: number) => n < 1024 ? `${n} B` : n < 1048576 ? `${(n / 1024).toFixed(1)} KB` : `${(n / 1048576).toFixed(1)} MB`;
export const artifactTitle = (a: Artifact) => a.name || a.kind.replaceAll("-", " ");
export const artifactFilename = (a: Artifact) => {
  const suffix: Record<string, string> = { "image/png": ".png", "image/jpeg": ".jpg", "image/webp": ".webp", "image/gif": ".gif", "image/svg+xml": ".svg", "application/json": ".json", "text/plain": ".txt" };
  return (a.name || `${a.kind}-${a.id.slice(0, 12)}${suffix[a.mime_type ?? ""] || ".bin"}`).replace(/[\\/\x00-\x1f\x7f]/g, "_").slice(0, 200);
};

export async function artifactPage(run: ArtifactRun, artifact: Artifact, offset: number) {
  const result = await run({ kind: "artifact_read", artifact: artifact.id, offset: String(offset), limit: 32768 }) as { base64: string; next: string | null };
  if (!result || typeof result.base64 !== "string" || result.base64.length > 44000) throw new Error("Invalid artifact byte page");
  const bytes = Uint8Array.from(atob(result.base64), c => c.charCodeAt(0));
  const end = offset + bytes.length;
  if (bytes.length > 32768 || end > artifact.bytes || (!bytes.length && end !== artifact.bytes) || (result.next === null ? end !== artifact.bytes : result.next !== String(end) || end >= artifact.bytes)) throw new Error("Incomplete or invalid artifact byte page");
  return { bytes, next: result.next };
}

export async function artifactBytes(run: ArtifactRun, artifact: Artifact, current: () => boolean = () => true): Promise<Uint8Array<ArrayBuffer>> {
  if (!Number.isSafeInteger(artifact.bytes) || artifact.bytes < 0 || artifact.bytes > MAX_ARTIFACT_DOWNLOAD) throw new Error("Artifact exceeds the 8 MB download limit");
  const bytes = new Uint8Array(artifact.bytes); let offset = 0;
  do {
    if (!current()) throw new Error("Artifact selection changed");
    const page = await artifactPage(run, artifact, offset);
    if (!current()) throw new Error("Artifact selection changed");
    bytes.set(page.bytes, offset); offset += page.bytes.length;
    if (page.next === null) break;
  } while (offset < bytes.length);
  const digest = Array.from(new Uint8Array(await crypto.subtle.digest("SHA-256", bytes)), b => b.toString(16).padStart(2, "0")).join("");
  if (digest !== artifact.sha256) throw new Error("Artifact integrity mismatch");
  if (!current()) throw new Error("Artifact selection changed");
  return bytes;
}
