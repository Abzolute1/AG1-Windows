import { useEffect, useRef, useState } from "react";
import type { Command, JsonValue } from "./generated/protocol";
import { Button } from "./components/ui/button";
type Artifact = { id: string; kind: string; bytes: number; sha256: string; created_at: string; mime_type?: string };
type Catalog = { artifacts: Artifact[]; next: JsonValue };
export function ArtifactsPane({ run }: { run: (command: Command) => Promise<JsonValue> }) {
  const [catalog, setCatalog] = useState<Catalog | null>(null), [selected, setSelected] = useState(""), [page, setPage] = useState(""), [next, setNext] = useState<string | null>("0"), [error, setError] = useState("");
  const [imageUrl, setImageUrl] = useState<string | null>(null), [imageBusy, setImageBusy] = useState(false);
  useEffect(() => { setImageUrl(null); }, [selected]);
  useEffect(() => () => { if (imageUrl) URL.revokeObjectURL(imageUrl); }, [imageUrl]);
  const generation = useRef(0), decoder = useRef(new TextDecoder());
  async function load(before: JsonValue = null) {
    try { setError(""); setCatalog(await run({ kind: "artifacts", action: "list", args: { before } }) as unknown as Catalog); } catch (e) { setError(String(e)); }
  }
  useEffect(() => { void load(); return () => { generation.current++; }; }, [run]);
  async function read() {
    const ticket = generation.current;
    try {
      setError("");
      const result = await run({ kind: "artifact_read", artifact: selected, offset: next ?? "0", limit: 32768 }) as { base64: string; next: string | null };
      if (ticket !== generation.current) return;
      setPage(decoder.current.decode(Uint8Array.from(atob(result.base64), c => c.charCodeAt(0)), { stream: result.next !== null })); setNext(result.next);
    } catch (e) { if (ticket === generation.current) { setPage(""); setError(String(e)); } }
  }
  async function previewImage() {
    const artifact = catalog?.artifacts.find(a => a.id === selected);
    if (!artifact || artifact.bytes > 8 * 1024 * 1024 || imageBusy) return;
    const ticket = generation.current; setImageBusy(true); setError("");
    try {
      const bytes = new Uint8Array(artifact.bytes); let offset: string | null = "0";
      while (offset !== null) {
        const result = await run({ kind: "artifact_read", artifact: selected, offset, limit: 32768 }) as { base64: string; next: string | null };
        if (ticket !== generation.current) return;
        const chunk = Uint8Array.from(atob(result.base64), c => c.charCodeAt(0));
        if (!chunk.length || Number(offset) + chunk.length > bytes.length || (result.next !== null && Number(result.next) !== Number(offset) + chunk.length)) throw new Error("Invalid image byte page");
        bytes.set(chunk, Number(offset)); if (result.next === null && Number(offset) + chunk.length !== bytes.length) throw new Error("Incomplete image"); offset = result.next;
      }
      const digest = Array.from(new Uint8Array(await crypto.subtle.digest("SHA-256", bytes)), b => b.toString(16).padStart(2, "0")).join("");
      if (digest !== artifact.sha256) throw new Error("Artifact image integrity mismatch");
      if (ticket === generation.current) setImageUrl(URL.createObjectURL(new Blob([bytes], { type: artifact.mime_type })));
    } catch (e) { if (ticket === generation.current) setError(String(e)); } finally { setImageBusy(false); }
  }
  const visual = catalog?.artifacts.find(a => a.id === selected && ["image/png", "image/jpeg", "image/webp", "image/gif"].includes(a.mime_type ?? "") && a.bytes <= 8 * 1024 * 1024);
  return <section><h3>Artifacts and retained outputs</h3>{error && <p role="alert">{error}</p>}<Button onClick={() => void load()}>Refresh artifact catalog</Button>{catalog?.artifacts.map(a => <article key={a.id}><Button onClick={() => { generation.current++; decoder.current = new TextDecoder(); setSelected(a.id); setNext("0"); setPage(""); }}>{a.kind} · {a.bytes} bytes</Button><small>{a.id} · {a.created_at} · SHA-256 {a.sha256}</small></article>)}{catalog?.next && <Button onClick={() => void load(catalog.next)}>Next artifacts</Button>}<label>Artifact ID<input value={selected} onChange={e => { generation.current++; decoder.current = new TextDecoder(); setSelected(e.target.value); setNext("0"); setPage(""); }} /></label><Button disabled={!selected || next === null} onClick={() => void read()}>{next === "0" ? "Read artifact bytes" : "Next artifact byte page"}</Button>{next === null && <p>End of artifact.</p>}{visual && <Button disabled={imageBusy} onClick={() => void previewImage()}>{imageBusy ? "Loading image…" : "Preview retained image"}</Button>}{imageUrl && <img style={{ maxWidth: "100%" }} src={imageUrl} alt="Retained artifact" />}<pre className="artifact-page">{page}</pre><p>Byte pages are integrity checked by the host. Images up to 8 MiB can be reassembled and verified for preview. Other binary pages are displayed as text with replacement characters.</p></section>;
}
