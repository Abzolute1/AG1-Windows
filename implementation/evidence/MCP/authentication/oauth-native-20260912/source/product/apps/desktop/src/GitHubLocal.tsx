import { useEffect, useRef, useState } from "react";
import type { Command, JsonValue } from "./generated/protocol";
import { Source } from "./GitHubWorkbench";
import { reviewError } from "./reviewErrors";
import { api } from "./bridge";
export type GitRun = (command: Command) => Promise<JsonValue>;
export function GitHubLocal({ repo, workspace, run, effect, changes, initialAction = "status", initialPullNumber, initialSha = "", attach }: { repo: string; workspace: string; run: GitRun; effect: GitRun; changes?: () => void; initialAction?: "status" | "clone" | "fetch_pr"; initialPullNumber?: number; initialSha?: string; attach?: (path: string) => void }) {
  const [action, setAction] = useState<string>(initialAction), [paths, setPaths] = useState(""), [message, setMessage] = useState(""), [name, setName] = useState(""), [email, setEmail] = useState(""), [branch, setBranch] = useState("");
  const [preview, setPreview] = useState<any>(null), [result, setResult] = useState<any>(null), [error, setError] = useState(""), [busy, setBusy] = useState(false);
  const [remoteSha, setRemoteSha] = useState(initialSha), [acknowledged, setAcknowledged] = useState(false);
  const [pullNumber, setPullNumber] = useState(initialPullNumber?.toString() ?? "");
  const [destination, setDestination] = useState("");
  const [pullReady, setPullReady] = useState(false);
  const [recovery, setRecovery] = useState<{ id: string; canRestore: boolean } | null>(null), [recoveryChoice, setRecoveryChoice] = useState("");
  const epoch = useRef(0), pending = useRef<string | null>(null), working = useRef(false);
  function dismiss() { const ticket = pending.current; pending.current = null; setPreview(null); setAcknowledged(false); if (ticket) void run({ kind: "files", action: "git_dismiss", args: { ticket } }).catch(() => {}); }
  useEffect(() => {
    ++epoch.current; setPreview(null); setResult(null); setError(""); setBusy(false); setPullReady(false); setRecovery(null); setRecoveryChoice(""); working.current = false;
    return () => { ++epoch.current; const ticket = pending.current; pending.current = null; if (ticket) void run({ kind: "files", action: "git_dismiss", args: { ticket } }).catch(() => {}); };
  }, [repo, workspace, run]);
  async function execute(confirm: boolean) {
    if (working.current || confirm && (!pending.current || preview?.action === "push" && !acknowledged)) return;
    working.current = true; setBusy(true); setError(""); const current = epoch.current;
    try {
      const ticket = pending.current;
      if (confirm) { pending.current = null; setPreview(null); }
      const args: Record<string, JsonValue> = { action: action === "pull" && !pullReady ? "fetch" : action, repo, folder: workspace };
      if (["stage", "unstage"].includes(action)) args.paths = paths.split("\n").filter(Boolean);
      if (action === "commit") Object.assign(args, { message, author_name: name, author_email: email });
      if (action === "branch_create") args.branch = branch;
      if (action === "checkout") Object.assign(args, { branch, target_sha: remoteSha });
      if (action === "fetch_pr") Object.assign(args, { pull_number: Number(pullNumber), expected_remote_sha: remoteSha });
      if (["fetch", "push", "clone", "pull"].includes(action)) Object.assign(args, { branch, expected_remote_sha: remoteSha });
      if (action === "clone") args.destination = destination;
      if (action === "recover") Object.assign(args, { recovery_id: recovery?.id ?? "", choice: recoveryChoice });
      const value = await (confirm ? effect({ kind: "files", action: "git_confirm", args: { ticket } }) : run({ kind: "files", action: "git_preview", args })) as any;
      if (current !== epoch.current) { if (!confirm && value?.ticket) void run({ kind: "files", action: "git_dismiss", args: { ticket: value.ticket } }).catch(() => {}); return; }
      if (confirm) {
        if (!value?.ok) throw new Error(value?.outcome === "unknown" ? `The Git outcome is uncertain. Inspect the destination, Changes and branch history before trying again.${typeof value.recovery_path === "string" ? ` Recovery files may remain at ${value.recovery_path}. They have not been automatically deleted.` : ""}` : value?.error ?? "Local Git did not apply this request.");
        setResult(value.result);
        if (value.result?.operation === "recovery_status" && value.result.outcome === "pending" && typeof value.result.recovery_id === "string") {
          setRecovery({ id: value.result.recovery_id, canRestore: value.result.can_restore === true }); setRecoveryChoice(""); setAction("recover");
        }
        if (value.result?.operation === "recover") { setRecovery(null); setRecoveryChoice(""); setAction("recovery_status"); }
        if (action === "pull" && value.result?.operation === "fetch" && value.result?.outcome === "applied" && value.result.repository === repo && value.result.remote_branch === `refs/heads/${branch}` && value.result.commit === remoteSha) setPullReady(true);
        if (value.result?.operation === "pull") setPullReady(false);
        if (value.result?.operation === "fetch_pr" && value.result?.outcome === "applied" && Number.isSafeInteger(value.result.pull_number) && value.result.pull_number > 0 && typeof value.result.commit === "string") {
          setAction("checkout"); setBranch(`pr/${value.result.pull_number}`); setRemoteSha(value.result.commit);
        }
      } else {
        if (typeof value?.ticket !== "string" || value.preview?.repository !== repo || value.preview?.folder !== workspace) throw new Error("The native Git target could not be verified.");
        pending.current = value.ticket; setPreview(value.preview); setResult(null); setAcknowledged(false);
      }
    } catch (e) { if (current === epoch.current) setError(localError(e)); }
    finally { if (current === epoch.current) { working.current = false; setBusy(false); } }
  }
  return <section className="github-workbench" aria-label="Local Git">
    <h3>Local Git</h3><p>{workspace}</p><p className="review-caption">Clone creates a new folder inside this granted root. Checkout preserves unrelated dirty files. Staging records exact file or link bytes, or an already-deleted tracked entry; it never follows symlink targets or changes working files. Linked worktrees also need Read & write access to the common repository folder in Chat folders. Filters and signing are not supported.</p>
    {changes && <button onClick={changes}>Open workspace changes</button>}
    <label>Local operation<select aria-label="Local Git operation" disabled={busy} value={action} onChange={e => { dismiss(); setResult(null); setPullReady(false); setAction(e.target.value); }}>
      {["status", "diff", "branches", "stage", "unstage", "commit", "branch_create", "fetch", "fetch_pr", "pull", "push", "clone", "checkout", "recovery_status", "recover"].map(op => <option key={op} value={op}>{op === "fetch_pr" ? "fetch pull request" : op === "pull" ? "pull (fast-forward only)" : op === "recovery_status" ? "inspect recovery" : op === "recover" ? "recover transition" : op.replaceAll("_", " ")}</option>)}
    </select></label>
    {["stage", "unstage"].includes(action) && <label>Exact relative paths, one per line<textarea aria-label="Git paths" disabled={busy} maxLength={16000} value={paths} onChange={e => { dismiss(); setPaths(e.target.value); }} /></label>}
    {action === "commit" && <><label>Commit message<textarea aria-label="Commit message" disabled={busy} maxLength={16000} value={message} onChange={e => { dismiss(); setMessage(e.target.value); }} /></label><label>Author name<input aria-label="Git author name" disabled={busy} maxLength={200} value={name} onChange={e => { dismiss(); setName(e.target.value); }} /></label><label>Author email<input aria-label="Git author email" disabled={busy} maxLength={200} value={email} onChange={e => { dismiss(); setEmail(e.target.value); }} /></label></>}
    {action === "branch_create" && <label>New branch<input aria-label="New branch name" disabled={busy} maxLength={200} value={branch} onChange={e => { dismiss(); setBranch(e.target.value); }} /></label>}
    {action === "checkout" && <><label>Target local branch<input aria-label="Checkout branch" disabled={busy} maxLength={200} value={branch} onChange={e => { dismiss(); setBranch(e.target.value); }} /></label><label>Reviewed target commit SHA<input aria-label="Checkout target SHA" disabled={busy} maxLength={40} value={remoteSha} onChange={e => { dismiss(); setRemoteSha(e.target.value); }} /></label><p>Fetch the reviewed commit first. An existing branch must already match this exact commit; a new branch is created at it. Conflicting dirty entries are refused. Originals and the index remain in a recovery directory; interrupted transitions are never automatically rolled back.</p></>}
    {action === "fetch_pr" && <><label>Pull request number<input aria-label="Git pull request number" disabled={busy} inputMode="numeric" maxLength={10} value={pullNumber} onChange={e => { dismiss(); setPullNumber(e.target.value); }} /></label><label>Reviewed pull request head SHA<input aria-label="Git pull request SHA" disabled={busy} maxLength={40} value={remoteSha} onChange={e => { dismiss(); setRemoteSha(e.target.value); }} /></label><p>First download this exact PR head into the selected repository. Working files and the local branch stay unchanged. After it succeeds, review a separate checkout confirmation before switching branches.</p></>}
    {["fetch", "push", "clone", "pull"].includes(action) && <><label>Remote branch<input aria-label="Git remote branch" disabled={busy} maxLength={200} value={branch} onChange={e => { dismiss(); setPullReady(false); setBranch(e.target.value); }} /></label><label>Reviewed remote head SHA<input aria-label="Reviewed remote SHA" disabled={busy} maxLength={40} value={remoteSha} onChange={e => { dismiss(); setPullReady(false); setRemoteSha(e.target.value); }} /></label><p>Use the exact 40-character head from GitHub’s branch view. All zeros means a new branch for push only. Fetch updates this remote-tracking ref; it does not pull into working files. Push requires fast-forward ancestry and may trigger CI. Transfers are bounded to 256 MiB per pack file.</p></>}
    {action === "pull" && <p role="status">{pullReady ? "Step 2 of 2: fetch completed. Review the exact working-file changes before confirming the fast-forward." : "Step 1 of 2: review and confirm the download first. Working files remain unchanged until a separate fast-forward confirmation."} Pull advances only the current same-named branch. Divergence and conflicting dirty files are refused; no merge, rebase or stash.</p>}
    {action === "recovery_status" && <p>Inspect the native record for this folder. Reading it does not restore files, clear Git locks or replay an earlier request.</p>}
    {action === "recover" && <>{recovery ? <><p>Native recovery: <code style={{ overflowWrap: "anywhere" }}>{recovery.id}</code></p><label>Recovery choice<select aria-label="Git recovery choice" disabled={busy} value={recoveryChoice} onChange={e => { dismiss(); setRecoveryChoice(e.target.value); }}><option value="">Choose what to recover…</option><option value="finish">Finish the retained target</option>{recovery.canRestore && <option value="restore">Restore the recorded starting state</option>}</select></label><p>Review a new exact request before anything changes. Later edits and unknown, live or replaced locks are refused. Retained files and any created branch are kept. No network request or automatic retry.</p></> : <button disabled={busy} onClick={() => { dismiss(); setAction("recovery_status"); }}>Inspect pending recovery</button>}</>}
    {action === "clone" && <><label>New relative folder<input aria-label="Clone destination" disabled={busy} maxLength={2047} value={destination} onChange={e => { dismiss(); setDestination(e.target.value); }} placeholder="repository-name" /></label><p>The parent must already exist inside {workspace}. Existing destinations are never merged or overwritten. No automatic submodule or LFS downloads. Checkout is limited to 10,000 entries and 256 MiB.</p></>}
    {error && <p role="alert">{error}</p>}
    {error && (error.includes("needs recovery") || error.includes(".ag1-git-recovery-")) && <button disabled={busy} onClick={() => { dismiss(); setAction("recovery_status"); setError(""); }}>Inspect recovery</button>}
    {preview ? <><LocalPreview preview={preview} />{preview.action === "push" && <label><input type="checkbox" disabled={busy} checked={acknowledged} onChange={e => setAcknowledged(e.target.checked)} />I reviewed the exact remote branch, commit and possible CI costs.</label>}<div className="artifact-actions"><button disabled={busy} onClick={dismiss}>Cancel Git request</button><button disabled={busy || preview.action === "push" && !acknowledged} onKeyDown={e => { if (e.key === "Enter") e.preventDefault(); }} onClick={() => void execute(true)}>Confirm local Git request</button></div></> : <button disabled={busy || action === "recover" && (!recovery || !recoveryChoice)} onClick={() => void execute(false)}>{busy ? "Working…" : "Review local Git request"}</button>}
    {result?.operation === "clone" && result?.outcome === "applied" && typeof result.path === "string" && <><p role="status">Repository cloned into {result.path}.</p>{attach && <button onClick={() => attach(result.path)}>Open cloned project</button>}</>}
    {result?.operation === "checkout" && <p role="status">{result.outcome === "unchanged" ? "Already at the reviewed branch and commit." : `Checked out ${result.branch} at ${result.commit}.`}{typeof result.recovery_path === "string" && ` Recovery files: ${result.recovery_path}`}</p>}
    {result?.operation === "pull" && <p role="status">{result.outcome === "unchanged" ? "Already at the reviewed remote commit." : `Fast-forwarded ${result.branch} to ${result.commit}.`}{typeof result.recovery_path === "string" && ` Recovery files: ${result.recovery_path}`}</p>}
    {result?.operation === "recover" && result.outcome === "applied" && <p role="status">{result.recovery_choice === "restore" ? "Recorded starting state restored." : "Retained target recovered."} {result.branch} · {result.commit}. All recovery copies are retained.</p>}
    {result?.operation === "recovery_status" && ["none", "complete"].includes(result.outcome) && <p role="status">No native Git transition is pending in this folder.</p>}
    {result?.operation === "fetch_pr" && result.outcome === "applied" && <p role="status">Pull request objects fetched. Review the checkout request to switch branches; nothing else runs automatically.</p>}
    {result && <Source value={result} label="Local Git result" />}
    {busy && ["fetch", "fetch_pr", "push", "clone", "pull"].includes(action) && <button onClick={() => void api.github("api_cancel").catch(e => setError(reviewError(e)))}>Cancel network Git request</button>}
  </section>;
}

export function LocalPreview({ preview }: { preview: any }) {
  const fields = [["Repository", preview.repository], ["Folder", preview.folder], ["Worktree metadata", preview.metadata?.linked ? preview.metadata.git?.path : undefined], ["Shared repository metadata", preview.metadata?.linked ? preview.metadata.common?.path : undefined], ["Destination", preview.destination], ["Branch", preview.remote_branch ?? preview.branch], ["Target branch", preview.target_branch], ["Target commit", preview.target_sha], ["Tracking ref", preview.tracking_ref], ["Reviewed remote commit", preview.expected_remote_sha], ["Local commit", preview.head], ["Retained recovery folder", preview.recovery_path]];
  return <section aria-label="Reviewed Git change">
    <h4>{String(preview.action).replaceAll("_", " ")} · review before applying</h4>
    {preview.account && <p>Account: @{preview.account.login} · {preview.account.host}</p>}
    <dl>{fields.filter(([, value]) => typeof value === "string" && value).map(([label, value]) => <div key={label}><dt>{label}</dt><dd style={{ overflowWrap: "anywhere" }}>{value}</dd></div>)}</dl>
    {preview.storage && <p className="review-caption">Git metadata and retained files: {Math.ceil(preview.storage.repository_bytes / 1048576)} MiB of {Math.floor(preview.storage.repository_limit / 1048576)} MiB. Private recovery storage: {Math.ceil(preview.storage.journal_bytes / 1048576)} MiB of {Math.floor(preview.storage.journal_limit / 1048576)} MiB. This request reserves additional space; retained records are never removed automatically.</p>}
    {preview.network_warning && <p>{preview.network_warning}</p>}
    {preview.recovery_warning && <p>{preview.recovery_warning}</p>}
    {typeof preview.choice === "string" && <p>Recovery choice: {preview.choice === "restore" ? "restore recorded starting state" : "finish retained target"}</p>}
    {Array.isArray(preview.retained_locks) && preview.retained_locks.length > 0 && <p>Reviewed stale locks to retain: {preview.retained_locks.join(", ")}</p>}
    {Array.isArray(preview.files) && preview.files.length > 0 && <><p>{preview.file_count ?? preview.files.length} exact selected entries{preview.files_clipped ? ` · first ${preview.files.length} shown` : ""}</p><ul>{preview.files.map((file: any, i: number) => <li key={i}><code style={{ whiteSpace: "pre-wrap", overflowWrap: "anywhere" }}>{file.path}</code>{file.operation && ` · ${file.operation}`}</li>)}</ul></>}
    {typeof preview.staged_diff === "string" && preview.staged_diff && <details open={preview.action === "commit"}><summary>Reviewed staged changes</summary>{typeof preview.display_limits === "string" && <p className="review-caption">{preview.display_limits}</p>}<Source value={preview.staged_diff} label="Reviewed staged diff" /></details>}
    <details><summary>Exact local Git details</summary><Source value={preview} label="Exact local Git preview" /></details>
  </section>;
}

function localError(error: unknown): string {
  const message = reviewError(error), limited = /^LimitExceeded\((.*)\)$/s.exec(message);
  if (limited) { try { const detail = JSON.parse(limited[1]); if (typeof detail === "string") return `Git limit reached: ${detail}`; } catch { /* Preserve the native error if it is not a quoted message. */ } }
  return message;
}
