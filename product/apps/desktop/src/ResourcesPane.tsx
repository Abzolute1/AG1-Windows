import { useEffect, useState } from "react";
import type { Command, JsonValue } from "./generated/protocol";
import { ResourceEditor } from "./ResourceEditor";
import { ResourceActions } from "./ResourceActions";
import { ExtensionCommands } from "./ExtensionUI";
import { Button } from "./components/ui/button";
type Resource = { name: string; description?: string; filePath?: string; path?: string; workspace_path?: string };
export function ResourcesPane({ run }: { run: (command: Command) => Promise<JsonValue> }) {
  const [editing, setEditing] = useState<string | null>(null);
  const [catalog, setCatalog] = useState<Record<string, Resource[]> | null>(null), [error, setError] = useState("");
  useEffect(() => { let cancelled = false; void run({ kind: "runtime", action: "resources", args: {} }).then(value => { if (!cancelled) setCatalog(value as unknown as Record<string, Resource[]>); }).catch(e => { if (!cancelled) setError(String(e)); }); return () => { cancelled = true; }; }, [run]);
  return <section aria-label="Resources"><h3>Skills, templates and extensions</h3>{error && <p role="alert">{error}</p>}{editing && <ResourceEditor key={editing} path={editing} run={run} close={() => setEditing(null)} />}{!catalog ? <p>Loading resources…</p> : ["skills", "prompts", "themes", "extensions", "diagnostics"].map(kind => <section key={kind}><h4>{kind}</h4>{catalog[kind]?.length ? catalog[kind].map((resource, i) => <article key={i}><strong>{resource.name ?? resource.path}</strong><p>{resource.description}</p><small>{resource.filePath ?? resource.path}</small>{resource.workspace_path && !editing && <Button onClick={() => setEditing(resource.workspace_path!)}>Edit {kind === "skills" ? "skill" : "template"}</Button>}{["skills", "prompts"].includes(kind) && <Button onClick={() => void run({ kind: "runtime", action: "client_action", args: { text: `${kind === "skills" ? "/skill:" : "/"}${resource.name}` } }).catch(e => setError(String(e)))}>Use {kind === "skills" ? "skill" : "template"}</Button>}</article>) : <p>None discovered.</p>}</section>)}<ResourceActions run={run} /><ExtensionCommands run={run} /><p>Resources are resolved in the isolated workspace and private host configuration. Extension trust and package installation require explicit host admission.</p></section>;
}
