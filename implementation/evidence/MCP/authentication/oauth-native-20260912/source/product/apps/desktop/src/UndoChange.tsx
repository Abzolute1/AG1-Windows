import { useEffect, useRef, useState } from "react";
import type { Command, JsonValue } from "./generated/protocol";
import { reviewError } from "./reviewErrors";
type Run = (command: Command) => Promise<JsonValue>;
type Outcome = { state: string; recovery?: string | null; detail?: string | null };
type Status = { allowed: boolean; reason?: string; recoverable?: boolean; outcome?: Outcome; operation?: string };

export function UndoChange({ id, path, run, effect }: { id: string; path: string; run: Run; effect: Run }) {
  const [status, setStatus] = useState<Status | null>(null), [confirm, setConfirm] = useState(false), [busy, setBusy] = useState(false), [error, setError] = useState("");
  const epoch = useRef(0), applying = useRef(false);
  async function refresh() {
    const ticket = ++epoch.current;
    try {
      const value = await run({ kind: "files", action: "change_undo_status", args: { id } }) as unknown as Status;
      if (ticket !== epoch.current) return;
      if (!value || typeof value.allowed !== "boolean") throw new Error("Undo availability could not be verified.");
      setStatus(value);
    } catch (e) { if (ticket === epoch.current) setError(reviewError(e)); }
  }
  useEffect(() => { setStatus(null); setConfirm(false); setError(""); void refresh(); return () => { epoch.current++; }; }, [id, run]);
  async function apply() {
    if (applying.current || !status || !status.allowed && !status.recoverable) return;
    applying.current = true; setBusy(true); setError("");
    const ticket = epoch.current;
    try {
      const outcome = await effect({ kind: "files", action: status.recoverable ? "change_undo_recover" : "change_undo", args: { id } }) as unknown as Outcome;
      if (ticket !== epoch.current) return;
      if (!outcome || !["completed", "not_applied", "recovery_required", "restored_postimage"].includes(outcome.state)) throw new Error("The Undo outcome is uncertain. Refresh its status before taking another action.");
      setStatus({ allowed: false, outcome }); setConfirm(false); await refresh();
    } catch (e) { if (ticket === epoch.current) { setError(reviewError(e)); setConfirm(false); await refresh(); } }
    finally { applying.current = false; setBusy(false); }
  }
  const outcome = status?.outcome;
  return <section className="undo-change" aria-label="Undo recorded file change">
    {error && <p role="alert">{error}</p>}
    {outcome && <p role="status">{outcome.state === "completed" ? "Undo completed." : outcome.state === "restored_postimage" ? "The displaced file was restored. Undo was not replayed." : outcome.state === "not_applied" ? "Undo was not applied." : "Undo needs recovery review. It will not be repeated automatically."}</p>}
    {outcome?.recovery && <details className="review-metadata"><summary>Recovery copy</summary><p>The displaced file is retained at <code>{outcome.recovery}</code>.</p>{outcome.detail && <p>{outcome.detail}</p>}</details>}
    {status?.reason && <p className="review-caption">{status.reason}</p>}
    {confirm ? <div role="group" aria-label="Confirm file Undo"><p>{status?.recoverable ? `Restore the displaced copy of ${path} into its empty path?` : status?.operation === "remove_created" ? `Undo creation of ${path}? A recovery copy will be retained.` : `Restore the exact bytes of ${path} from before this operation? Later changes will block Undo.`}</p>
      <div className="artifact-actions"><button disabled={busy} onClick={() => setConfirm(false)}>Keep current file</button><button disabled={busy} onKeyDown={e => { if (e.key === "Enter") e.preventDefault(); }} onClick={() => void apply()}>{busy ? "Applying…" : status?.recoverable ? "Restore displaced file" : "Undo this file change"}</button></div>
    </div> : <div className="artifact-actions">{(status?.allowed || status?.recoverable) && <button disabled={busy} onClick={() => setConfirm(true)}>{status.recoverable ? "Recover displaced file" : "Undo file change"}</button>}<button disabled={busy} onClick={() => void refresh()}>Refresh Undo status</button></div>}
  </section>;
}
