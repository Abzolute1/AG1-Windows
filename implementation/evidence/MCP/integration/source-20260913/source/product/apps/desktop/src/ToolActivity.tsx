import { memo, useState } from "react";
import { Check, ChevronRight, Copy, Github, FileText, Search, LoaderCircle, Terminal, X } from "lucide-react";
import type { ClientMessage, MessageBlock } from "./generated/protocol";
import { ChatBlocks } from "./ChatBlocks";
import { JournalImages, previewableImage, type ImageRun } from "./ImageAttachment";
import { GitHubResult } from "./GitHubResult";
import "./workflow.css";
type Call = Extract<MessageBlock, { kind: "tool" }>;
const labels: Record<string, string> = { read: "Read", ls: "List folder", grep: "Search", glob: "Find files", bash: "Run command", check: "Check", edit: "Edit", editPreview: "Preview edit", editApply: "Apply edit", append: "Append", write: "Write", webSearch: "Search the web", webFetch: "Fetch page", repo: "Inspect repository" };
export function toolTarget(call: Call): string {
  const args = call.arguments;
  if (!args || typeof args !== "object" || Array.isArray(args)) return "";
  if (call.name === "github") {
    for (const value of [args.repo, args.schemaId, (args.parameters as any)?.q, (args.parameters as any)?.operation]) if (typeof value === "string") return value.replace(/\s+/g, " ").slice(0, 240);
    return ["catalog", "schema", "repos.list", "repos.search", "repos.get", "repos.contents", "issues.list", "pulls.list"].includes(String(args.operation)) ? "" : String(args.operation ?? "").slice(0, 128);
  }
  for (const key of ["repo", "schemaId", "path", "filePath", "file_path", "command", "query", "pattern", "url", "glob", "operation", "action"]) {
    if (typeof args[key] === "string") return args[key].replace(/\s+/g, " ").slice(0, 240);
  }
  return "";
}
export function toolLabel(call: Call, completed = false): string {
  if (call.name.startsWith("mcp_")) return "MCP operation";
  if (call.name === "mcpDiscover") return completed ? "Discovered MCP capabilities" : "Discovering MCP capabilities";
  const a = call.arguments as Record<string, any> | null;
  const operation = typeof a?.operation === "string" ? a.operation.replaceAll("/", ".") : "";
  if (call.name === "github") {
    const names: Record<string, [string, string]> = { catalog: ["Discovering GitHub tools", "Discovered GitHub tools"], schema: ["Checking GitHub schema", "Checked GitHub schema"], "repos.list": ["Listing repositories", "Listed repositories"], "repos.search": ["Searching repositories", "Searched repositories"], "repos.get": ["Reading repository", "Read repository"], "repos.contents": ["Browsing repository files", "Browsed repository files"], "issues.list": ["Listing issues", "Listed issues"], "pulls.list": ["Listing pull requests", "Listed pull requests"] };
    return names[operation]?.[completed ? 1 : 0] ?? (completed ? "Used GitHub" : "Using GitHub");
  }
  if (call.name === "inspect") return a?.query ? (completed ? "Searched source" : "Searching source") : a?.glob ? (completed ? "Found files" : "Finding files") : "Read file";
  const names: Record<string, [string, string]> = { bash: ["Running command", "Ran command"], grep: ["Searching source", "Searched source"], find: ["Finding files", "Found files"], work: ["Updating work record", "Updated work record"], evidence: ["Retrieving evidence", "Retrieved evidence"], capability: ["Using specialist tools", "Used specialist tools"], ui: ["Inspecting preview", "Inspected preview"], preview: ["Managing preview", "Managed preview"] };
  return names[call.name]?.[completed ? 1 : 0] ?? labels[call.name] ?? call.name;
}
function prominentGitHub(message: ClientMessage): boolean {
  if (message.tool?.failed) return true;
  try { const text = message.blocks.filter(b => b.kind === "text").map(b => b.text).join("\n"); if (text.length > 512000) return false; const v = JSON.parse(text); return ["applied", "unknown", "unchanged"].includes(v?.outcome) || !!v?.data?.download || !!v?.recovery_path; } catch { return false; }
}
export function previewText(text: string): string | null {
  if (text.length > 256 * 1024) return null;
  try {
    const value = JSON.parse(text);
    if (typeof value?.path !== "string" || !Array.isArray(value.hunks) || value.hunks.length > 1000 || !value.hunks.every((h: any) => typeof h?.diff === "string")) return null;
    return [`--- ${value.path}`, `+++ ${value.path}`, ...value.hunks.map((h: any) => h.diff), ...(Array.isArray(value.warnings) ? value.warnings.filter((w: unknown) => typeof w === "string") : []), ...(typeof value.error === "string" ? [value.error] : [])].join("\n");
  } catch { return null; }
}
/** Literal output is data, never Markdown/HTML. Mount it only when requested. */
export const ToolOutput = memo(function ToolOutput({ message, run, loading = false, hideImages = false }: { message: ClientMessage; run: ImageRun; loading?: boolean; hideImages?: boolean }) {
  const [all, setAll] = useState(false), [wrap, setWrap] = useState(false), [copied, setCopied] = useState(false), [error, setError] = useState("");
  const raw = message.blocks.filter(b => b.kind === "text").map(b => b.text).join("\n");
  const decoded = message.tool?.name === "editPreview" ? previewText(raw) : null;
  const [showRaw, setShowRaw] = useState(false);
  const text = !showRaw && decoded !== null ? decoded : raw;
  // Bound both line count and line length; a single minified file must not freeze the view.
  const prefix = text.slice(0, 12000).split("\n").slice(0, 32).join("\n");
  const clipped = prefix.length < text.length;
  return <div className="tool-output-view">
    <h4>{decoded !== null ? "Preview" : "Output"}</h4>
    {message.tool?.name === "runtimeObservation" && <p className="muted">Recorded observation. Storage success does not verify the reported outcome.</p>}
    {message.tool?.name === "memory" && <p className="muted">Memory records are context; source freshness and verification are reported separately.</p>}
    {decoded !== null && <button onClick={() => setShowRaw(!showRaw)}>{showRaw ? "Show diff" : "Show raw result"}</button>}
    <div className="tool-output-toolbar"><span>{loading ? "Loading full output…" : `${text.length.toLocaleString()} characters`}</span><button aria-pressed={wrap} onClick={() => setWrap(!wrap)}>Wrap lines</button><button aria-label="Copy tool output" onClick={() => void navigator.clipboard.writeText(text).then(() => { setCopied(true); setError(""); }).catch(() => setError("Could not copy. Select the output to copy it manually."))}>{copied ? <Check size={14} /> : <Copy size={14} />}{copied ? "Copied" : "Copy"}</button></div>
    {text && <pre role="region" className={`tool-output-text ${wrap ? "wrap" : ""}`} tabIndex={0} aria-label="Tool output"><code>{all ? text : prefix}</code></pre>}
    {clipped && <button className="output-more" onClick={() => setAll(!all)}>{all ? "Show less" : "Show full output"}</button>}
    <ChatBlocks run={run} blocks={message.blocks.filter(b => b.kind !== "text" && !(hideImages && b.kind === "attachment" && previewableImage(b.media_type)))} />
    {error && <p role="alert">{error}</p>}
  </div>;
});
function ToolEntry({ call, results, active, run, onExpand, loading }: { call?: Call; results: ClientMessage[]; active: boolean; run: ImageRun; onExpand: (id: string) => void; loading: (id: string) => boolean }) {
  const [open, setOpen] = useState(false), [argumentsOpen, setArgumentsOpen] = useState(false);
  const failed = results.some(m => m.tool?.failed), target = call ? toolTarget(call) : "";
  const source = results.flatMap(m => m.blocks).find(b => b.kind === "unknown" && b.value && typeof b.value === "object" && !Array.isArray(b.value) && b.value.kind === "mcp_result");
  const capability = source?.kind === "unknown" && source.value && typeof source.value === "object" && !Array.isArray(source.value) ? source.value.capability : undefined;
  const label = typeof capability === "string" ? `MCP · ${capability.slice(0, 128)}` : call ? toolLabel(call, results.length > 0 && !failed) : results[0]?.tool?.name || "Tool output";
  const Icon = call?.name === "github" ? Github : ["read", "inspect", "write", "edit"].includes(call?.name ?? "") ? FileText : ["grep", "find", "webSearch"].includes(call?.name ?? "") ? Search : Terminal;
  return <div className={`tool-entry ${failed ? "failed" : ""}`}>
    <button className="tool-entry-toggle" aria-expanded={open} onClick={() => { setOpen(!open); if (!open) results.forEach(m => onExpand(m.id)); }}>{failed ? <X size={14} /> : active && !results.length ? <LoaderCircle size={14} className="spin" /> : <Icon size={14} />}<span>{label}</span>{target && <span className="tool-target" title={target}>{target}</span>}{failed && <small>Failed</small>}{!active && !results.length && <small>No result</small>}<ChevronRight size={12} className={open ? "expanded" : ""} /></button>
    {open && <div className="tool-entry-body">
      {call && <><button className="tool-arguments-toggle" aria-expanded={argumentsOpen} onClick={() => setArgumentsOpen(!argumentsOpen)}>Arguments</button>{argumentsOpen && <pre className="tool-arguments">{JSON.stringify(call.arguments, null, 2)}</pre>}</>}
      {results.map(m => call?.name === "github" && !prominentGitHub(m) && !["catalog", "schema"].includes(String((call.arguments as any)?.operation))
        ? <GitHubResult key={m.id} inline call={call} message={m} expand={() => onExpand(m.id)}><ToolOutput message={m} run={run} loading={loading(m.id)} /></GitHubResult>
        : <ToolOutput key={m.id} message={m} run={run} loading={loading(m.id)} hideImages={!m.tool?.failed} />)}
      {!results.length && <p className="muted">{active ? "Waiting for output…" : "No output recorded."}</p>}
    </div>}
  </div>;
}
export function ActivityGroup({ messages, active, compact, run, onExpand, loading, renderAssistant, time, status }: { messages: ClientMessage[]; active: boolean; compact: boolean; run: ImageRun; onExpand: (id: string) => void; loading: (id: string) => boolean; renderAssistant?: (m: ClientMessage) => React.ReactNode; time?: string; status?: string }) {
  const [expanded, setExpanded] = useState<boolean | null>(null);
  const open = expanded ?? (active || !compact);
  const calls = messages.flatMap(m => m.role === "assistant" ? m.blocks.filter((b): b is Call => b.kind === "tool") : []);
  const results = messages.filter(m => m.role === "toolResult");
  const ids = new Set(calls.map(c => c.call_id));
  const matched = (call: Call) => results.filter(m => m.tool?.call_id === call.call_id);
  const failures = results.filter(m => m.tool?.failed).length;
  const count = calls.length + results.filter(m => !m.tool?.call_id || !ids.has(m.tool.call_id)).length;
  const images = [...new Set(results.filter(m => !m.tool?.failed).flatMap(m => m.blocks.flatMap(b => b.kind === "attachment" && previewableImage(b.media_type) ? [b.reference] : [])))];
  return <><div className="work-summary response-work"><button type="button" className="work-toggle" disabled={!messages.length} aria-expanded={!!messages.length && open} onClick={() => setExpanded(!open)}>{active && <LoaderCircle size={13} className="spin" />}<span>{active ? time ? `Working for ${time}` : "Working…" : time ? `Worked for ${time}` : "Work details"}</span>{!!messages.length && <ChevronRight size={12} className={open ? "expanded" : ""} />}{count > 0 && <small>{count} {count === 1 ? "tool" : "tools"}</small>}{failures > 0 && <small className="danger-text">{failures} failed</small>}</button>
    {active && status && <div className="work-phase" role="status">{status}</div>}
    {open && !!messages.length && <div className="work-log">{messages.map(m => <div key={m.id} className="work-step">{m.role === "assistant" && <>{renderAssistant?.(m)}{m.blocks.filter((b): b is Call => b.kind === "tool").map(call => <ToolEntry key={call.call_id} call={call} results={matched(call)} active={active} run={run} onExpand={onExpand} loading={loading} />)}</>}{m.role === "toolResult" && (!m.tool?.call_id || !ids.has(m.tool.call_id)) && <ToolEntry results={[m]} active={false} run={run} onExpand={onExpand} loading={loading} />}</div>)}</div>}
  </div>{calls.filter(call => call.name === "github").flatMap(call => matched(call).filter(m => prominentGitHub(m)).map(m => <GitHubResult key={m.id} call={call} message={m} expand={() => onExpand(m.id)}><ToolOutput message={m} run={run} loading={loading(m.id)} /></GitHubResult>))}{images.length > 0 && <div className="assistant-message image-results" aria-label="Image results">{images.map(reference => <JournalImages key={reference} reference={reference} run={run} inline />)}</div>}</>;
}
