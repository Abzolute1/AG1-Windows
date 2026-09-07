import { useEffect, useState } from "react";
import type { Command, JsonValue } from "./generated/protocol";
import { Button } from "./components/ui/button";
export function ResourceActions({ run }: { run: (command: Command) => Promise<JsonValue> }) {
  const [capabilities, setCapabilities] = useState<{ createSkill: boolean; managePackage: boolean } | null>(null), [error, setError] = useState("");
  const [name, setName] = useState(""), [description, setDescription] = useState(""), [source, setSource] = useState(""), [scope, setScope] = useState("project"), [busy, setBusy] = useState(false);
  useEffect(() => { let closed = false; void run({ kind: "runtime", action: "resource_actions", args: {} }).then(v => { if (!closed) setCapabilities(v as typeof capabilities); }).catch(e => { if (!closed) setError(String(e)); }); return () => { closed = true; }; }, [run]);
  async function execute(name: string, args: JsonValue) {
    setBusy(true); setError("");
    try { await run({ kind: "runtime", action: "resource_action", args: { name, arguments: args } }); } catch (e) { setError(String(e)); } finally { setBusy(false); }
  }
  return <section><h3>Create a skill</h3>{error && <p role="alert">{error}</p>}<form onSubmit={e => { e.preventDefault(); void execute("createSkill", { name, description }); }}><label>Skill name<input required value={name} onChange={e => setName(e.target.value)} /></label><label>Description<textarea required value={description} onChange={e => setDescription(e.target.value)} /></label><Button disabled={busy || !capabilities?.createSkill} type="submit">Create skill</Button></form>
    <h3>Packages</h3>{!capabilities?.managePackage && <p>Package helpers are not configured on this host.</p>}<label>Package source<input placeholder="npm:package@version or Git source" value={source} onChange={e => setSource(e.target.value)} /></label><label>Resource scope<select value={scope} onChange={e => setScope(e.target.value)}><option value="project">Project</option><option value="user">Private host</option><option value="temporary">Temporary</option></select></label>{["install", "update", "remove"].map(action => <Button key={action} disabled={busy || !source || !capabilities?.managePackage} onClick={() => void execute("managePackage", { action, source, scope })}>{action}</Button>)}<div>{[true, false].map(enabled => <Button key={String(enabled)} disabled={busy || !source} onClick={() => void execute("configurePackage", { source, enabled })}>{enabled ? "Enable project package" : "Disable project package"}</Button>)}</div><p>Enable packages after installation to include their resources in the next session. Disabling preserves installed files.</p><p>Review the native approval before installation. Installed package scripts run with the host account's privileges. Results and failures are retained in Activity and Artifacts.</p>
  </section>;
}
