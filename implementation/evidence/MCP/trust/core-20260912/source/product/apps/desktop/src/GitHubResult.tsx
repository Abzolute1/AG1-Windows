import { GitHubDownload } from "./GitHubDownload";
import { useContext, useState } from "react";
import { GitHubReviewContext } from "./GitHubPullReview";
import { api, errorMessage } from "./bridge";
import type { ClientMessage, MessageBlock } from "./generated/protocol";
type Call = Extract<MessageBlock, { kind: "tool" }>;
const names: Record<string, string> = { "issues.create": "Issue created", "issues.comment": "Comment published", "pulls.create": "Pull request created", "pulls.merge": "Pull request merged", "pulls.review": "Review submitted", "actions.dispatch": "Workflow dispatch accepted", "actions.rerun": "Workflow rerun accepted", "actions.cancel": "Workflow cancellation accepted", "releases.create": "Release created", "projects.update_field": "Project field updated", "git.checkout": "Branch checked out", "git.clone": "Repository cloned", "git.fetch": "Remote commit fetched", "git.fetch_pr": "Pull request fetched", "git.push": "Commit pushed", "git.pull": "Branch fast-forwarded", "git.recover": "Git state recovered" };
export function GitHubResult({ call, message, expand, children, inline = false }: { call: Call; message: ClientMessage; expand: () => void; children?: React.ReactNode; inline?: boolean }) {
  const [open, setOpen] = useState(false), [error, setError] = useState("");
  const remoteReview = useContext(GitHubReviewContext);
  const args = call.arguments as Record<string, unknown> | null;
  const operation = typeof args?.operation === "string" ? args.operation : "GitHub operation";
  if (["catalog", "schema"].includes(operation)) return null;
  const text = message.blocks.filter(b => b.kind === "text").map(b => b.text).join("\n");
  let value: any = null;
  if (text.length <= 512000) { try { value = JSON.parse(text); } catch { /* Journal/artifact-backed output still has a useful entry point. */ } }
  const uncertain = value?.outcome === "unknown", failed = message.tool?.failed || uncertain;
  const accepted = value?.outcome === "applied" && !failed;
  const local = operation.startsWith("git.");
  const label = uncertain ? local ? "Local Git outcome uncertain" : "GitHub outcome uncertain" : failed ? local ? "Local Git operation failed" : "GitHub operation failed" : accepted ? names[operation] ?? "GitHub change accepted" : value?.outcome === "unchanged" ? "Already at the reviewed Git state" : "GitHub result";
  const data = local ? value : value?.data;
  const number = data?.number ?? (args?.parameters as any)?.number;
  const canReview = remoteReview && !failed && operation.startsWith("pulls.") && typeof args?.repo === "string" && /^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(args.repo) && Number.isSafeInteger(number) && number > 0;
  const errorText = value?.message ?? (local ? value?.error : null);
  const candidate = data?.html_url ?? data?.url;
  let url: string | null = null;
  if (typeof candidate === "string" && candidate.length <= 2048) { try { const parsed = new URL(candidate); if (parsed.origin === "https://github.com" && !parsed.username && !parsed.password) url = parsed.href; } catch { /* Do not open untrusted origins. */ } }
  const rows = Array.isArray(data) ? data : Array.isArray(data?.items) ? data.items : Array.isArray(data?.workflow_runs) ? data.workflow_runs : null;
  return <section className={`github-result-card ${failed ? "failed" : ""}`} aria-label="GitHub result">
    <strong>{label}{Number.isSafeInteger(data?.number) ? ` · #${data.number}` : ""}</strong>
    <p>{operation.slice(0, 128)}{typeof args?.repo === "string" ? ` · ${args.repo.slice(0, 256)}` : ""}</p>
    {typeof data?.title === "string" && <p>{data.title.slice(0, 500)}</p>}
    {rows && <p>{rows.length} items on this page{value.next ? " · more available" : ""}</p>}
    {local && typeof data?.branch === "string" && <p>{data.branch.slice(0, 512)}{typeof data.commit === "string" ? ` · ${data.commit.slice(0, 64)}` : ""}</p>}
    {local && typeof data?.recovery_path === "string" && <p style={{ overflowWrap: "anywhere" }}>Recovery files: {data.recovery_path.slice(0, 4096)}</p>}
    {uncertain && <p>{local ? "Inspect local changes, branch history and retained recovery files before retrying." : "Inspect GitHub before repeating this change."}</p>}
    {failed && typeof errorText === "string" && <p>{errorText.slice(0, 1000)}</p>}
    {canReview && <button onClick={() => remoteReview.open({ repo: args.repo as string, number })}>Review PR files and checks</button>}
    <div className="github-actions">{url && <button onClick={() => void api.github("open_url", { url }).catch(e => setError(errorMessage(e)))}>Open on GitHub</button>}{!inline && <button aria-expanded={open} onClick={() => { if (!open) expand(); setOpen(v => !v); }}>{open ? "Hide GitHub details" : "Inspect GitHub result"}</button>}</div>
    {!failed && data?.download && !data?.base64 && <GitHubDownload value={data.download} scope={message.id} />}
    {(inline || open) && children}
    {error && <p role="alert">{error}</p>}
  </section>;
}
