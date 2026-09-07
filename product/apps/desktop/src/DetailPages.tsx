import { useRef, useState } from "react";
import type { Command, JsonValue } from "./generated/protocol";
import { Button } from "./components/ui/button";
export function DetailPages({ section, run }: { section: string; run: (command: Command) => Promise<JsonValue> }) {
  const [page, setPage] = useState<{ text: string; next: string | null; sha256: string; bytes: number } | null>(null), [error, setError] = useState("");
  const decoder = useRef(new TextDecoder());
  async function read(next = false) {
    try {
      setError(""); if (!next) decoder.current = new TextDecoder();
      const data = await run({ kind: "runtime", action: "detail_page", args: { section, offset: next ? page?.next ?? "0" : "0", ...(next ? { sha256: page!.sha256 } : {}) } }) as { base64: string; next: string | null; sha256: string; bytes: number };
      setPage({ ...data, text: decoder.current.decode(Uint8Array.from(atob(data.base64), c => c.charCodeAt(0)), { stream: data.next !== null }) });
    } catch (e) { setError(`${String(e)}. Reload the first page if the recorded state changed.`); }
  }
  return <details className="detail-pages"><summary>Complete {section} record</summary><Button onClick={() => void read()}>Read first page</Button>{page?.next && <Button onClick={() => void read(true)}>Next record page</Button>}{error && <p role="alert">{error}</p>}{page && <><small>{page.bytes} bytes · SHA-256 {page.sha256}</small><pre>{page.text}</pre></>}</details>;
}
