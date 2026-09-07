import { useState } from "react";
import type { Command, JsonValue } from "./generated/protocol";
import { Button } from "./components/ui/button";
export function RecoveryControls({ run }: { run: (command: Command) => Promise<JsonValue> }) {
  const [command, setCommand] = useState(""), [inspection, setInspection] = useState<JsonValue>(null), [error, setError] = useState("");
  async function execute(action: string, args: JsonValue) { try { setError(""); setInspection(await run({ kind: "runtime", action, args })); } catch (e) { setError(String(e)); } }
  return <section><h3>Recovery</h3><Button onClick={() => void execute("inspect_recovery", {})}>Inspect interrupted work</Button><label>Original command ID<input value={command} onChange={e => setCommand(e.target.value)} /></label><Button disabled={!command} onClick={() => void execute("client_effect_result", { command })}>Inspect client effect receipt</Button><Button onClick={() => void execute("recover_acknowledgement", { command })}>Reconcile stored acknowledgement</Button>{error && <p role="alert">{error}</p>}{inspection !== null && <pre className="artifact-page">{JSON.stringify(inspection, null, 2)}</pre>}<p>Reconciliation reads an already committed result. Interrupted effects require host inspection; this action never repeats a tool or declares an unknown effect successful.</p></section>;
}
