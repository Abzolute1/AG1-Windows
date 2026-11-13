import { useEffect, useRef, useState } from "react";
import { errorMessage } from "./bridge";
import { MCPDiscover } from "./MCPDiscover";
import { MCPPermissions } from "./MCPPermissions";
import "./mcp.css";

type Draft = { name: string; protocol: string; auth_profile: string | null; provenance: { source: "manual" }; configuration: any };
type Record = { id: string; revision: number; draft: Draft; availability: any; local_access?: string };
type Props = { scope: string; run: (args: any) => Promise<any>; back: () => void };
const empty = (): Draft => ({ name: "", protocol: "2026-07-28", auth_profile: null, provenance: { source: "manual" }, configuration: { transport: "http", url: "", loopback_development: false } });
const kinds = [["tool", "Tools"], ["resource", "Resources"], ["resourceTemplate", "Templates"], ["prompt", "Prompts"]];
function Source({ value }: { value: unknown }) { return <pre className="mcp-source">{JSON.stringify(value, null, 2)}</pre>; }
function mcpError(error: unknown) {
  const text = errorMessage(error);
  try {
    const native = JSON.parse(text.replace(/ \(command [0-9a-f-]+\)$/, "")).error;
    const messages: { [key: string]: string } = {
      Conflict: "The server changed. Review its current settings and try again.",
      PermissionDenied: "This action is no longer allowed. Check the server's credentials and permissions.",
      Cancelled: "The operation was cancelled.",
      TimedOut: "The operation timed out. Try again when the server is available.",
      QueueFull: "Another MCP operation is still running. Try again when it finishes.",
      NotFound: "This server or connection is no longer available.",
    };
    if (Object.hasOwn(messages, native)) return messages[native];
    const message = typeof native === "string" && native.match(/^(?:Unavailable|InvalidInput|LimitExceeded)\((".*")\)$/s);
    if (message) return JSON.parse(message[1]) as string;
  } catch { /* Other errors already carry a user-readable message. */ }
  return text;
}

export function MCPManager({ scope, run, back }: Props) {
  const [section, setSection] = useState("installed");
  const [view, setView] = useState<any>(null), [error, setError] = useState(""), [busy, setBusy] = useState(false), [notice, setNotice] = useState("");
  const [selected, setSelected] = useState<string | null>(null), [draft, setDraft] = useState<Draft | null>(null), [editing, setEditing] = useState<Record | null>(null);
  const [importReview, setImportReview] = useState<any>(null), [importAck, setImportAck] = useState(false);
  const importPending = useRef<string | null>(null);
  const [localAccess, setLocalAccess] = useState("readOnlyDirectory");
  const [argumentsText, setArguments] = useState("[]"), [environment, setEnvironment] = useState("{}"), [secretNames, setSecretNames] = useState("");
  const [review, setReview] = useState<any>(null), [acknowledged, setAcknowledged] = useState(false), [removing, setRemoving] = useState<Record | null>(null);
  const [kind, setKind] = useState("tool"), [catalog, setCatalog] = useState<any>(null), [detail, setDetail] = useState<any>(null);
  const mounted = useRef(0), working = useRef(false), pending = useRef<string | null>(null);
  async function invoke(args: any) { const ticket = mounted.current; const data = await run(args); if (ticket !== mounted.current) throw new Error("The conversation changed."); return data; }
  async function refresh(ticket = mounted.current) { const data = await invoke({ action: "list" }); if (ticket === mounted.current) setView(data); }
  async function action(work: () => Promise<void>) {
    if (working.current) return; working.current = true; setBusy(true); setError(""); setNotice("");
    const ticket = mounted.current;
    try { await work(); if (ticket === mounted.current) await refresh(ticket); }
    catch (e) {
      if (ticket === mounted.current) {
        setError(mcpError(e));
        // Failed native operations may still retire credentials/connections or
        // advance configuration revisions. Refresh before enabling another action.
        try { await refresh(ticket); }
        catch { if (ticket === mounted.current) setView(null); }
      }
    }
    finally { if (ticket === mounted.current) { working.current = false; setBusy(false); } }
  }
  useEffect(() => {
    const ticket = ++mounted.current; setSection("installed"); setView(null); setReview(null); setImportReview(null); setImportAck(false); setDraft(null); setSelected(null); setCatalog(null); setDetail(null); working.current = false;
    void refresh(ticket).catch(e => { if (ticket === mounted.current) setError(mcpError(e)); });
    const timer = window.setInterval(() => { if (!working.current) void refresh(ticket).catch(() => {}); }, 2000);
    return () => { const imported = importPending.current; importPending.current = null; if (imported) void run({ action: "import_dismiss", review: imported }).catch(() => {}); ++mounted.current; window.clearInterval(timer); const id = pending.current; pending.current = null; if (id) void run({ action: "dismiss", review: id }).catch(() => {}); };
  }, [scope, run]);
  const servers: any[] = view?.servers ?? [];
  const windowsNative = view?.windowsNative === true;
  const row = servers.find(r => r.record.id === selected), record: Record | undefined = row?.record;
  const oauth = row?.authentication?.target?.kind === "oauth";
  const connections: any[] = (view?.connections ?? []).filter((c: any) => c.serverId === selected);
  const ready = connections.find(c => c.health === "ready");
  useEffect(() => { setCatalog(null); setDetail(null); }, [ready?.id]);
  function edit(value: Record | null) {
    if (pending.current) void dismiss().catch(() => {});
    const d = value ? { ...value.draft, provenance: { source: "manual" as const } } : empty();
    // Windows does not have AG1's Bubblewrap isolation. Start its manual
    // configuration at the only mode the native host can honestly run.
    setLocalAccess(value?.local_access ?? (value ? "hostAccount" : windowsNative ? "hostAccount" : "readOnlyDirectory"));
    setDraft(d); setEditing(value); setArguments(JSON.stringify(d.configuration.arguments ?? [], null, 2)); setEnvironment(JSON.stringify(d.configuration.public_environment ?? {}, null, 2)); setSecretNames((d.configuration.secret_environment ?? []).join("\n")); setReview(null); setError("");
  }
  async function save() {
    if (!draft) return;
    const d = structuredClone(draft);
    if (d.configuration.transport === "local") {
      d.configuration.arguments = JSON.parse(argumentsText); d.configuration.public_environment = JSON.parse(environment); d.configuration.secret_environment = secretNames.split(/\s+/).filter(Boolean);
      if (!Array.isArray(d.configuration.arguments) || d.configuration.arguments.some((v: any) => typeof v !== "string")) throw new Error("Arguments must be a JSON array of strings.");
      if (!d.configuration.public_environment || Array.isArray(d.configuration.public_environment) || typeof d.configuration.public_environment !== "object" || Object.values(d.configuration.public_environment).some(v => typeof v !== "string")) throw new Error("Public environment must be a JSON object of strings.");
    }
    const access = d.configuration.transport === "local" ? { local_access: localAccess } : {};
    const result = await invoke(editing ? { action: "edit", id: editing.id, revision: editing.revision, draft: d, ...access } : { action: "add", draft: d, ...access });
    setSelected(editing?.id ?? result.ids[0]); setDraft(null); setEditing(null);
  }
  async function prepare(test: boolean) {
    if (!record) return;
    const r = await invoke({ action: "review", id: record.id, revision: record.revision, test }).catch(error => {
      if (record.draft.auth_profile && row.authentication?.status !== "connected") {
        throw new Error(oauth ? "OAuth credentials are missing, expired or logged out. Sign in again or refresh credentials, then review the connection again." : "Private credentials are missing, expired or logged out. Use Replace private values, then review the connection again.");
      }
      throw error;
    });
    pending.current = r.review; setReview(r); setAcknowledged(false);
  }
  async function dismiss() { const id = pending.current; pending.current = null; setReview(null); if (id) await invoke({ action: "dismiss", review: id }); }
  async function confirm() { const id = pending.current; pending.current = null; setReview(null); if (id) await invoke({ action: "confirm", review: id }); }
  async function load(k: string, offset = 0) {
    if (!ready) return;
    const value = await invoke({ action: "catalog", connection: ready.id, kind: k, offset });
    setKind(k); setCatalog(value); setDetail(null);
  }
  return <section className={`mcp-manager${section === "discover" ? " mcp-manager-discover" : ""}`} aria-label="MCP manager">
    <header className="mcp-heading"><div><h1>MCP</h1><p>Connect tools and services to AG1.</p></div><button onClick={back}>Back to chat</button></header>
    <nav className="mcp-tabs" aria-label="MCP sections"><button aria-current={section === "installed" ? "page" : undefined} disabled={busy} onClick={() => setSection("installed")}>Installed</button><button aria-current={section === "discover" ? "page" : undefined} disabled={busy || !!draft || !!review || !!importReview} onClick={() => setSection("discover")}>Discover</button></nav>
    {section === "installed" && <p>Installed servers are saved configurations. Connecting and approving a server operation are separate choices.</p>}
    {error && <p role="alert" className="inline-error">{error}</p>}
    {notice && <p role="status">{notice}</p>}
    {!view && !error ? <p role="status">Loading MCP servers…</p> : busy && <p role="status">Updating MCP…</p>}
    {section === "discover" ? <MCPDiscover invoke={invoke} action={action} busy={busy} added={(id, message) => { setSelected(id); setNotice(message); setSection("installed"); }} manual={() => { setSection("installed"); edit(null); }} /> : <>
    <div className="mcp-toolbar"><span>Current scope: {!view ? "Loading…" : view.scope?.kind === "generalChat" ? "General chat" : "Current workspace"}</span><button disabled={busy || !!importReview} onClick={() => edit(null)}>Add server</button><button disabled={busy || !view?.projectImportAvailable || !!importReview || !!draft} title="Review the current project’s .mcp.json; no server starts automatically" onClick={() => void action(async () => { const r = await invoke({ action: "import_review" }); importPending.current = r.review; setImportReview(r); setImportAck(false); })}>Import project config</button><button disabled={busy} onClick={() => void action(async () => {})}>Refresh</button></div>
    {importReview && <section aria-label="Review project MCP import" className="mcp-review"><h2>Review project servers</h2><p>{importReview.message}</p><p>Source: {importReview.source}</p><p>SHA-256: {importReview.sha256}</p><Source value={importReview.drafts} /><label className="checkbox-label"><input type="checkbox" checked={importAck} onChange={e => setImportAck(e.target.checked)} />I reviewed these project definitions for saving inactive.</label><div className="mcp-actions"><button disabled={busy} onClick={() => void action(async () => { const review = importPending.current; importPending.current = null; setImportReview(null); if (review) await invoke({ action: "import_dismiss", review }); })}>Cancel import</button><button disabled={busy || !importAck} onClick={() => void action(async () => { const review = importPending.current; importPending.current = null; setImportReview(null); if (review) { const saved = await invoke({ action: "import_confirm", review }); setSelected(saved.ids[0]); setNotice(saved.message); } })}>Save imported servers inactive</button></div></section>}
    {draft ? <form className="mcp-form" onSubmit={e => { e.preventDefault(); void action(save); }}>
      <h2>{editing ? "Edit server" : "Add server"}</h2>
      <label>Display name<input required maxLength={256} value={draft.name} onChange={e => setDraft({ ...draft, name: e.target.value })} /></label>
      <label>Connection type<select value={draft.configuration.transport} onChange={e => setDraft({ ...draft, configuration: e.target.value === "local" ? { transport: "local", executable: "", arguments: [], cwd: "", public_environment: {}, secret_environment: [] } : { transport: "http", url: "", loopback_development: false } })}><option value="http">Remote · Streamable HTTP</option><option value="local">Local · stdio</option></select></label>
      {draft.configuration.transport === "http" ? <><label>MCP URL<input required type="url" maxLength={4096} placeholder="https://service.example/mcp" value={draft.configuration.url} onChange={e => setDraft({ ...draft, configuration: { ...draft.configuration, url: e.target.value } })} /></label><label className="checkbox-label"><input type="checkbox" checked={draft.configuration.loopback_development} onChange={e => setDraft({ ...draft, configuration: { ...draft.configuration, loopback_development: e.target.checked } })} />Allow this exact loopback development endpoint</label></> : <>
        <label>Executable · absolute path<input required maxLength={4096} placeholder="/usr/bin/python3" value={draft.configuration.executable} onChange={e => setDraft({ ...draft, configuration: { ...draft.configuration, executable: e.target.value } })} /></label>
        <label>Arguments · JSON array<textarea rows={4} maxLength={16000} value={argumentsText} onChange={e => setArguments(e.target.value)} /></label>
        <label>Working directory · absolute path<input required maxLength={4096} value={draft.configuration.cwd} onChange={e => setDraft({ ...draft, configuration: { ...draft.configuration, cwd: e.target.value } })} /></label>
        <label>Local process access<select value={localAccess} onChange={e => setLocalAccess(e.target.value)}><option value="readOnlyDirectory" disabled={windowsNative}>Read-only directory · no network{windowsNative ? " (Linux only)" : ""}</option><option value="hostAccount">Host account · full account access</option></select></label>
        <p>{localAccess === "readOnlyDirectory" ? windowsNative ? "Windows-native AG1W will not emulate this Linux-only sandbox. Choose host-account access to run the reviewed server, or leave it inactive." : "The canonical working directory and system runtimes are readable. Other user directories are hidden, and writes and networking are blocked. This requires Linux user namespaces and bubblewrap. Runtimes requiring writes or sockets cannot run in this mode." : "This command runs with your operating-system account permissions, including filesystem and network access. Workspace selection and tool rules do not contain the process."}</p>
        <label>Public environment · JSON object<textarea rows={3} maxLength={16000} value={environment} onChange={e => setEnvironment(e.target.value)} /></label><p>Use public configuration values only. Do not enter tokens or passwords. No shell expansion or credential environment is inherited.</p>
        <label>Private environment names · one per line<textarea rows={3} maxLength={4096} placeholder="API_TOKEN" value={secretNames} onChange={e => setSecretNames(e.target.value)} /></label><p>Enter names only. After saving, use native private entry to supply their values.</p>
      </>}
      <label>Protocol revision<select value={draft.protocol} onChange={e => setDraft({ ...draft, protocol: e.target.value })}><option>2026-07-28</option><option>2025-11-25</option></select></label>
      <p>Saving keeps the server inactive. Editing disconnects it, removes old credentials and resets availability. After saving, use native private entry or supported OAuth sign-in to configure authentication.</p>
      <div className="mcp-actions"><button type="button" disabled={busy} onClick={() => { setDraft(null); setEditing(null); }}>Cancel</button><button className="primary-button" disabled={busy}>Save inactive</button></div>
    </form> : <div className="mcp-layout"><aside aria-label="Installed MCP servers">{servers.map(r => <button className={selected === r.record.id ? "selected" : ""} key={r.record.id} disabled={busy || !!review} onClick={() => { setSelected(r.record.id); setCatalog(null); setDetail(null); }}><strong>{r.record.draft.name}</strong><small>{r.record.draft.configuration.transport === "local" ? "Local" : "Remote"} · {r.availableHere ? "Available here" : "Unavailable here"}</small></button>)}{view && !servers.length && <p>No servers installed. Add a local or remote MCP server to get started.</p>}</aside>
      <div className="mcp-details">{record ? <><h2>{record.draft.name}</h2><p className="mcp-identity">Server ID: {record.id}</p><p>{row.availableHere ? "Enabled for this scope" : "Disabled for this scope"} · Configuration revision {record.revision}</p>
        <div className="mcp-actions"><button disabled={busy} onClick={() => void action(async () => { await invoke({ action: "availability", id: record.id, revision: record.revision, enabled: !row.availableHere, target: "current" }); })}>{row.availableHere ? "Disable here" : "Enable here"}</button><button disabled={busy || !row.availableHere || connections.some(c => ["ready", "connecting", "stopping"].includes(c.health))} onClick={() => void action(() => prepare(false))}>Connect</button><button disabled={busy || connections.some(c => ["ready", "connecting", "stopping"].includes(c.health))} onClick={() => void action(() => prepare(true))}>Test connection</button><button disabled={busy} onClick={() => edit(record)}>Edit configuration</button></div>
        {connections.map(c => <div className="mcp-connection" key={c.id}><span>{c.waitingForInput ? "Waiting for your input" : c.health}{c.inspectionOnly ? " · Inspection only; reconnect after enabling to use with the model" : " · This chat"}{c.diagnostic ? ` · ${c.diagnostic}` : ""}</span><button disabled={busy} onClick={() => void action(async () => { await invoke({ action: "disconnect", connection: c.id }); setCatalog(null); setDetail(null); })}>Disconnect</button></div>)}
        {review && <section className="mcp-review" aria-label="Review MCP connection"><h3>Review connection · {review.server.draft.name}</h3><p>{review.warning}</p><p>{review.purpose}</p><Source value={{ configuration: review.server.draft.configuration, scope: review.scope, executableSha256: review.executableSha256 }} /><label className="checkbox-label"><input type="checkbox" checked={acknowledged} onChange={e => setAcknowledged(e.target.checked)} />I reviewed the exact target and connection effects.</label><div className="mcp-actions"><button disabled={busy} onClick={() => void action(dismiss)}>Cancel review</button><button className="primary-button" disabled={busy || !acknowledged} onClick={() => void action(confirm)}>Connect reviewed target</button></div></section>}
        <p className="mcp-endpoint">{record.draft.configuration.transport === "local" ? `Local · ${record.draft.configuration.executable}` : `Remote · ${record.draft.configuration.url}`}</p><details><summary>Overview and configuration</summary><Source value={record.draft} /></details>
        <section aria-label="MCP authentication"><h3>Authentication</h3>
          <p>{row.authentication?.status === "connected" ? (oauth ? "OAuth credentials saved" : "Private credentials saved") : row.authentication?.status === "authenticating" ? "Signing in" : row.authentication?.status === "revoked" ? "Logged out locally" : row.authentication?.status === "expired" ? "Credentials expired" : record.draft.auth_profile ? "Authentication required for this configuration" : "No private credentials configured"}</p>
          <p>Account: {row.authentication?.account ?? (oauth ? "Identity unavailable" : "Not verified for a manually entered credential")}.</p>
          {oauth && <><p>Authorization server: {row.authentication.target.issuer}<br />Resource: {row.authentication.target.resource}</p><p>Granted scopes: {row.authentication.scopes?.join(", ") || "None reported"}.</p></>}
          <p>Updating credentials disconnects this server in every scope. Connecting and approving operations remain separate choices.</p>
          <div className="mcp-actions">
            {record.draft.configuration.transport === "http" && !record.draft.configuration.loopback_development && <button disabled={busy || !!review || !view?.oauthAvailable} onClick={() => void action(async () => { setCatalog(null); setDetail(null); const result = await invoke({ action: "oauth", id: record.id, revision: record.revision, refresh: false }); setNotice(result.message); })}>{oauth ? "Sign in again" : "Sign in with OAuth"}</button>}
            {oauth && ["connected", "expired"].includes(row.authentication?.status) && <button disabled={busy || !!review || !view?.oauthAvailable} onClick={() => void action(async () => { setCatalog(null); setDetail(null); const result = await invoke({ action: "oauth", id: record.id, revision: record.revision, refresh: true }); setNotice(result.message); })}>Refresh credentials</button>}
            <button disabled={busy || !!review || !view?.privateEntryAvailable || (record.draft.configuration.transport === "local" && !record.draft.configuration.secret_environment?.length)} onClick={() => void action(async () => { await invoke({ action: "private_entry", id: record.id, revision: record.revision }); setCatalog(null); setDetail(null); })}>{oauth ? "Use a manual token" : record.draft.auth_profile ? "Replace private values" : "Enter private values"}</button>
            {record.draft.auth_profile && <button disabled={busy || !!review} onClick={() => void action(async () => { const result = await invoke({ action: "logout", id: record.id, revision: record.revision }); setCatalog(null); setDetail(null); setNotice(result.message); })}>Log out locally</button>}
          </div>
          {record.draft.configuration.transport === "local" && !record.draft.configuration.secret_environment?.length && <p>Add private environment names in Edit configuration first.</p>}
          {record.draft.configuration.transport === "http" && <p>OAuth uses your system browser and a registered public client ID. Registration requirements and requested scopes are reviewed in the native dialog.</p>}
          {!view?.oauthAvailable && record.draft.configuration.transport === "http" && <p>Native OAuth sign-in is unavailable in this window.</p>}
          {!view?.privateEntryAvailable && <p>Native private entry is unavailable in this window.</p>}
        </section>
        <details><summary>Workspace availability</summary><p>Availability determines which scopes can discover this server. Capability rules and local process access are separate permissions.</p><Source value={record.availability} /><button disabled={busy} onClick={() => void action(async () => { await invoke({ action: "availability", id: record.id, revision: record.revision, enabled: !record.availability.all_workspaces, target: "all_workspaces" }); })}>{record.availability.all_workspaces ? "Remove all-workspace availability" : "Make available in all workspaces"}</button><button disabled={busy} onClick={() => void action(async () => { await invoke({ action: "availability", id: record.id, revision: record.revision, enabled: false, target: "everywhere" }); })}>Disable everywhere</button></details>
        <section aria-label="MCP capabilities"><h3>Capabilities</h3><div className="mcp-tabs">{kinds.map(([id, label]) => <button key={id} disabled={busy || !ready} aria-pressed={kind === id && !!catalog} onClick={() => void action(() => load(id))}>{label}</button>)}</div>{!ready && <p>Connect or test the server to inspect its capabilities.</p>}{catalog && <><p>{catalog.total} entries · Protocol {catalog.binding.protocol}</p><div className="mcp-capabilities">{catalog.items.map((item: any) => <button key={item.key} disabled={busy || !ready} onClick={() => void action(async () => { setDetail(await invoke({ action: "detail", connection: ready?.id, kind, key: item.key })); })}><strong>{item.name}</strong><span>{item.description}</span></button>)}</div>{catalog.next != null && <button disabled={busy} onClick={() => void action(() => load(kind, catalog.next))}>Next capabilities</button>}</>}{detail && <section><h4>Capability details</h4><p>{detail.classificationBasis} {detail.approval}</p>{detail.unavailableReason && <p role="status">{detail.unavailableReason}</p>}<details><summary>Complete capability details</summary><Source value={detail} /></details></section>}</section>
        <MCPPermissions key={`${scope}:${record.id}`} scope={scope} server={record} policy={row.trustPolicy} detail={detail} connection={ready?.id} kind={kind} busy={busy} run={invoke} perform={action} notice={setNotice} />
        <details><summary>Diagnostics</summary><p>Connection health and native error categories are shown above. Server messages and descriptions are untrusted. No server operation is performed by these inspection views.</p></details>
        <button className="danger-text" disabled={busy} onClick={() => setRemoving(record)}>Remove server</button>{removing?.id === record.id && <div className="mcp-review"><p>Remove this configuration and revoke its connections in every scope? Conversation evidence will remain.</p><button disabled={busy} onClick={() => setRemoving(null)}>Keep server</button><button disabled={busy} onClick={() => void action(async () => { await invoke({ action: "remove", id: removing.id, revision: removing.revision }); setRemoving(null); setSelected(null); })}>Confirm removal</button></div>}
      </> : <p>Select a server to manage its connection, capabilities, and availability.</p>}</div></div>}
    </>}
  </section>;
}
