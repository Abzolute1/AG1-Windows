import { useEffect, useRef, useState } from "react";
import { ExtensionSettings } from "./ExtensionSettings";
import { Button } from "./components/ui/button";
import type { Command, JsonValue } from "./generated/protocol";
type Run = (command: Command) => Promise<JsonValue>;
type Event = { id: string; method: string; extension: string; arguments: { args?: JsonValue[]; key?: string; content?: string[] } };
type Ui = { pending: Event | null; events: Event[] };
export function ExtensionUI({ activity, run, onEditor }: { activity: JsonValue; run: Run; onEditor: (value: string, paste?: boolean) => void }) {
  const ui = (activity as { extension_ui?: Ui })?.extension_ui;
  const [answer, setAnswer] = useState(""), [error, setError] = useState("");
  const seen = useRef(new Set<string>());
  useEffect(() => {
    for (const event of ui?.events ?? []) {
      if (seen.current.has(event.id)) continue;
      seen.current.add(event.id);
      if (["ui.setEditorText", "ui.pasteToEditor"].includes(event.method)) onEditor(String(event.arguments.args?.[0] ?? ""), event.method === "ui.pasteToEditor");
    }
    if (seen.current.size > 256) seen.current = new Set(ui?.events.map(e => e.id));
  }, [ui?.events, onEditor]);
  const pending = ui?.pending;
  useEffect(() => { setAnswer(String(pending?.arguments.args?.[1] ?? "")); setError(""); }, [pending?.id]);
  async function respond(value: JsonValue) {
    if (!pending) return;
    try { await run({ kind: "runtime", action: "extension_answer", args: { id: pending.id, value } }); } catch (e) { setError(String(e)); }
  }
  return <>
    {pending && <section className="extension-question" aria-label="Extension question"><strong>{String(pending.arguments.args?.[0] ?? "Extension request")}</strong><small>{pending.extension}</small>{error && <p role="alert">{error}</p>}
      {pending.method === "ui.confirm" ? <><p>{String(pending.arguments.args?.[1] ?? "")}</p><Button onClick={() => void respond(true)}>Confirm</Button><Button onClick={() => void respond(false)}>Decline</Button></> : pending.method === "ui.select" ? <>{(pending.arguments.args?.[1] as string[] ?? []).map(option => <Button key={option} onClick={() => void respond(option)}>{option}</Button>)}<Button onClick={() => void respond(null)}>Cancel selection</Button></> : <form onSubmit={e => { e.preventDefault(); void respond(answer); }}><textarea aria-label="Extension answer" value={answer} onChange={e => setAnswer(e.target.value)} /><Button type="submit">Answer</Button><Button type="button" onClick={() => void respond(null)}>Cancel answer</Button></form>}
    </section>}
    {!!ui?.events.length && <details className="extension-notifications"><summary>Extension activity ({ui.events.length})</summary>{ui.events.map(event => <div key={event.id}><strong>{event.method.replace("ui.", "")}</strong><pre>{event.arguments.content?.join("\n") ?? (event.arguments.args ?? []).map(v => typeof v === "string" ? v : JSON.stringify(v)).join(" · ")}</pre></div>)}</details>}
  </>;
}
export function ExtensionCommands({ run }: { run: Run }) {
  const [commands, setCommands] = useState<{ name: string; invocationName: string; description?: string }[]>([]), [error, setError] = useState(""), [args, setArgs] = useState("");
  useEffect(() => { let closed = false; void run({ kind: "runtime", action: "extension_catalog", args: {} }).then(v => { if (!closed) setCommands((v as unknown as { commands: typeof commands }).commands); }).catch(e => { if (!closed) setError(String(e)); }); return () => { closed = true; }; }, [run]);
  return <section><ExtensionSettings run={run} /><h3>Admitted extension commands</h3>{error && <p role="alert">{error}</p>}<label>Command arguments<input value={args} onChange={e => setArgs(e.target.value)} /></label>{commands.length ? commands.map(command => <Button key={command.invocationName} title={command.description} onClick={() => void run({ kind: "runtime", action: "extension_command", args: { name: command.invocationName, arguments: args } }).catch(e => setError(String(e)))}>{command.name}</Button>) : <p>No extension commands admitted.</p>}</section>;
}
