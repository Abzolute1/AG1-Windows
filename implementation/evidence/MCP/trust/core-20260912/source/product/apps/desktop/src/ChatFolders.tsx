import { useEffect, useRef, useState } from "react";
import { FolderOpen } from "lucide-react";
import { api, errorMessage } from "./bridge";
export type FolderMode = "read_only" | "read_write";
export type FolderSelection = { revision: number; primary: string; folders: { path: string; mode: FolderMode; available?: boolean }[] };
export type FolderChange = { revision: number; action: "add" | "mode" | "primary" | "remove"; path: string; mode?: FolderMode };

export function ChatFolders({ selection, blocked, change, chooseFolder = api.chooseChatFolder }: { selection?: FolderSelection; blocked?: string; change: (request: FolderChange) => Promise<FolderSelection>; chooseFolder?: () => Promise<string | null> }) {
  const [value, setValue] = useState(selection), [path, setPath] = useState(""), [mode, setMode] = useState<FolderMode>("read_only");
  const [busy, setBusy] = useState(false), [error, setError] = useState("");
  const pending = useRef(false);
  useEffect(() => { setValue(selection); }, [selection]);
  async function browse() {
    if (blocked || pending.current) return;
    pending.current = true; setBusy(true); setError("");
    try { const path = await chooseFolder(); if (path) setPath(path); }
    catch (error) { setError(errorMessage(error)); }
    finally { pending.current = false; setBusy(false); }
  }
  async function update(action: FolderChange["action"], target: string, access?: FolderMode) {
    if (!value || blocked || pending.current) return;
    pending.current = true; setBusy(true); setError("");
    try { const next = await change({ revision: value.revision, action, path: target, ...(access ? { mode: access } : {}) }); setValue(next); if (action === "add") setPath(""); }
    catch (error) { setError(errorMessage(error)); }
    finally { pending.current = false; setBusy(false); }
  }
  return <div className="chat-folders">
    <p>Choose where this chat can work. Your conversation stays here when you change folders.</p>
    {blocked && <p className="folder-notice" role="status">{blocked}</p>}
    {error && <p className="inline-error" role="alert">{error}</p>}
    {!value ? <p role="status">Loading chat folders…</p> : <>
      <div className="folder-grants">{value.folders.map(folder => <section key={folder.path} aria-label={`Folder ${folder.path}`}>
        <div className="folder-grant-path"><FolderOpen size={16} /><span>{folder.path}{folder.available === false && <small> · Unavailable — remove and add again</small>}</span>{folder.path === value.primary && <small>Working folder</small>}</div>
        <div className="folder-grant-controls">
          <select aria-label={`Access for ${folder.path}`} value={folder.mode} disabled={busy || !!blocked} onChange={e => void update("mode", folder.path, e.target.value as FolderMode)}><option value="read_only">Read only</option><option value="read_write">Read &amp; write</option></select>
          {folder.path !== value.primary && <><button disabled={busy || !!blocked} onClick={() => void update("primary", folder.path)}>Make working folder</button><button aria-label={`Remove ${folder.path}`} disabled={busy || !!blocked} onClick={() => void update("remove", folder.path)}>Remove</button></>}
        </div>
      </section>)}</div>
      <form onSubmit={e => { e.preventDefault(); if (path.trim()) void update("add", path.trim(), mode); }}>
        <label htmlFor="chat-folder-path">Add a folder</label>
        <div className="folder-path-input"><input id="chat-folder-path" placeholder="/home/you/projects/my-project" value={path} disabled={busy || !!blocked} onChange={e => setPath(e.target.value)} autoComplete="off" spellCheck={false} /><button type="button" disabled={busy || !!blocked} onClick={() => void browse()}>Browse…</button></div>
        <div className="folder-add-actions"><select aria-label="New folder access" value={mode} disabled={busy || !!blocked} onChange={e => setMode(e.target.value as FolderMode)}><option value="read_only">Read only</option><option value="read_write">Read &amp; write</option></select><button className="primary-button" disabled={busy || !!blocked || !path.trim() || value.folders.length >= 16}>Add folder</button></div>
      </form>
      <p className="folder-footnote">Saved for this chat only. Read &amp; write still follows your approval mode. Choose another working folder before removing the current one.</p>
      <details><summary>What these permissions cover</summary><p>The working folder is the default for relative paths, Git, and project tools. Folder permissions apply to project file tools and agent shell writes. Agent shell commands retain host-wide read access; this is not a full read sandbox. The manual terminal and trusted plugins have their own host permissions. AG1’s private storage cannot be added. Folders cannot overlap.</p></details>
    </>}
  </div>;
}
