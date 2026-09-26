import { useState } from "react";
import type { JsonValue } from "./generated/protocol";
import type { Artifact, ArtifactRun } from "./artifactData";
import { ArtifactPreview } from "./ArtifactsPane";

export function MCPResult({ value, run }: { value: Record<string, JsonValue>; run: ArtifactRun }) {
  const [open, setOpen] = useState(false);
  const raw = value.original as Record<string, JsonValue> | null;
  const artifact: Artifact | null = raw && typeof raw.id === "string" && typeof raw.kind === "string" && typeof raw.bytes === "number" && Number.isSafeInteger(raw.bytes) && raw.bytes >= 0 && typeof raw.sha256 === "string" && /^[a-f0-9]{64}$/.test(raw.sha256)
    ? { id: raw.id, kind: raw.kind, bytes: raw.bytes, sha256: raw.sha256, created_at: "", mime_type: "application/json" } : null;
  const refs = Array.isArray(value.references) ? value.references.slice(0, 8) : [];
  return <section className="mcp-result" aria-label="MCP result source">
    <p><strong>MCP · {String(value.capability ?? "Operation")}</strong> · {String(value.outcome ?? "unknown").replaceAll("_", " ")}</p>
    <p className="muted">Server {String(value.server ?? "unknown")}</p>
    {refs.map((r, i) => r && typeof r === "object" && !Array.isArray(r) ? <div className="mcp-resource-card" key={i}>
      <strong>{String(r.name || "Resource")}</strong><code>{String(r.uri ?? "")}</code>
      <small>{r.retained ? "Content retained in the original response" : "Reference only · not fetched"}</small>
    </div> : null)}
    {value.complete !== true && <p>Continuation input and private state were omitted from this retained response.</p>}
    {artifact && <button aria-expanded={open} onClick={() => setOpen(!open)}>{open ? "Hide original MCP response" : "Inspect original MCP response"}</button>}
    {open && artifact && <ArtifactPreview artifact={artifact} run={run} />}
  </section>;
}
