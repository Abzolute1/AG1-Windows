export type Recovery = {
  owner: "native" | "type_script";
  stage: string;
  epoch: number;
  state: string | null;
  transfer: { archive: string; original_state: string } | null;
  rollback: { archive: string; destination: string } | null;
};
export function RecoveryPanel({ record, onClose }: { record: Recovery; onClose: () => void }) {
  return <section aria-label="Workspace recovery" className="card" style={{ margin: 16, padding: 16 }}>
    <h2>Workspace recovery</h2>
    <p>{record.stage === "active" ? "Another client may still hold this workspace. Close it cleanly before reopening." : "The workspace is paused for migration. Reopen it after host recovery finishes."}</p>
    <dl>
      <dt>Recorded owner</dt><dd>{record.owner === "native" ? "Native" : "TypeScript"}</dd>
      <dt>Recovery stage</dt><dd>{record.stage}</dd>
      {record.state && <><dt>Current data</dt><dd>{record.state}</dd></>}
      {record.transfer && <><dt>Original data</dt><dd>{record.transfer.original_state}</dd><dt>Migration archive</dt><dd>{record.transfer.archive}</dd></>}
      {record.rollback && <><dt>Native recovery archive</dt><dd>{record.rollback.archive}</dd><dt>New TypeScript destination</dt><dd>{record.rollback.destination}</dd></>}
    </dl>
    <p>Keep these archives. Uncertain effects require inspection before any retry. Recovery does not undo file changes or external actions.</p>
    <button onClick={onClose}>Close recovery details</button>
  </section>;
}
