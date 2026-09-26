import { useEffect, useState } from "react";
import { artifactSize, artifactTitle, type ArtifactCatalog, type ArtifactRun } from "./artifactData";

export function TurnOutputs({ turn, revision, run, review }: { turn: string; revision: string; run: ArtifactRun; review: (turn: string) => void }) {
  const [catalog, setCatalog] = useState<ArtifactCatalog | null>(null), [failed, setFailed] = useState(false);
  useEffect(() => {
    let current = true; setCatalog(null); setFailed(false);
    void run({ kind: "artifacts", action: "list", args: { turn, before: null } }).then(value => {
      if (!current) return;
      const page = value as unknown as ArtifactCatalog;
      if (!page || !Array.isArray(page.artifacts) || page.artifacts.length > 20 || page.artifacts.some(a => !a || typeof a.id !== "string" || typeof a.kind !== "string" || !Number.isSafeInteger(a.bytes) || a.bytes < 0)) throw new Error("Invalid native outputs");
      setCatalog(page);
    }).catch(() => { if (current) setFailed(true); });
    return () => { current = false; };
  }, [turn, revision, run]);
  if (!failed && !catalog?.artifacts.length) return null;
  return <section className="turn-change-card" aria-label="Retained outputs for this turn">
    {failed ? <p>Turn outputs could not load.</p> : <><strong>{catalog!.artifacts.length} retained {catalog!.artifacts.length === 1 ? "output" : "outputs"}{catalog!.next !== null ? " on this page" : ""}</strong>
      <ul>{catalog!.artifacts.slice(0, 3).map(a => <li key={a.id}><span>{artifactTitle(a)}</span><small>{artifactSize(a.bytes)}</small></li>)}</ul></>}
    <button onClick={() => review(turn)}>Review turn outputs</button>
  </section>;
}
