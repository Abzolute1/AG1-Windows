import type { ClientMessage, MessageBlock } from "./generated/protocol";
import { ActivityMark } from "./ActivityMark";
import { pickWorkingVerb, type WorkingVerbKind } from "./workingVerbs";

type Call = Extract<MessageBlock, { kind: "tool" }>;
export type ActivityDescription = { kind?: WorkingVerbKind; label: string; key: string; paused?: boolean };
function commandKind(command: unknown): WorkingVerbKind {
  if (typeof command !== "string") return "executing";
  // Deliberately conservative: a mention in an echo, a filename or user prose
  // must not make AG1 claim that it is running tests or building software.
  const text = command.trim().replace(/^(?:[A-Za-z_]\w*=(?:"[^"]*"|'[^']*'|[^\s]+)\s+)+/, "");
  if (/^(?:(?:cargo|go)\s+test\b|(?:npm|pnpm|yarn|bun)\s+(?:run\s+)?(?:test|check)(?:\s|:|$)|(?:pytest|vitest|jest|ctest)(?:\s|$)|python(?:3)?\s+(?:-m\s+(?:pytest|unittest)\b|(?:[^\s/]+\/)*(?:verify|test(?:_[\w-]+)?)\.py(?:\s|$)))/.test(text)) return "testing";
  if (/^(?:(?:cargo|go)\s+build\b|(?:npm|pnpm|yarn|bun)\s+(?:run\s+)?build(?:\s|:|$)|cmake\s+--build\b|(?:make|ninja)(?:\s|$))/.test(text)) return "building";
  return "executing";
}
function toolKind(call: Call): WorkingVerbKind {
  const args = call.arguments && typeof call.arguments === "object" && !Array.isArray(call.arguments) ? call.arguments : {};
  switch (call.name) {
    case "webSearch": case "grep": case "find": return "searching";
    case "inspect": return args.query || args.glob ? "searching" : "reading";
    case "read": case "ls": case "glob": case "webFetch": case "repo": return "reading";
    case "write": case "append": return "writing";
    case "edit": case "editApply": case "apply_patch": case "str_replace_editor": return "editing";
    case "editPreview": return "reading";
    case "check": return "testing";
    case "bash": case "exec": case "runCommand": return commandKind(args.command ?? args.cmd);
    default: return "executing";
  }
}
const labels: Record<WorkingVerbKind, string> = {
  thinking: "Thinking", reading: "Reading", searching: "Searching", writing: "Writing",
  editing: "Editing", building: "Building", testing: "Running tests", executing: "Running tools", general: "Working",
};
/** Inputs are native display state and actual unresolved tool calls. Never
 * classify from the user's prompt or carry a completed tool into the next wait. */
export function describeActivity(status: string, messages: ClientMessage[], live?: { id: string; committed: boolean; blocks: MessageBlock[] }): ActivityDescription {
  const generation = live?.id ?? messages.filter(m => m.role === "assistant").at(-1)?.id ?? "starting";
  if (status === "Thinking" || status === "Generating") {
    const output = live && !live.committed ? live.blocks.filter(b => b.kind === "text" || b.kind === "thinking").at(-1) : undefined;
    const kind = output?.kind === "text" && output.text.trim() ? "writing" : status === "Thinking" ? "thinking" : "writing";
    return { kind, label: kind === "writing" ? "Generating response" : "Thinking", key: generation };
  }
  if (status === "Preparing request" || status === "Generating operation" || status === "Processing prompt") {
    return { kind: status === "Generating operation" ? "writing" : "general", label: status, key: generation };
  }
  if (status === "Running tools") {
    const complete = new Set(messages.filter(m => m.role === "toolResult").map(m => m.tool?.call_id));
    const call = messages.filter(m => m.role === "assistant").flatMap(m => m.blocks.filter((b): b is Call => b.kind === "tool")).find(c => !complete.has(c.call_id));
    const kind = call ? toolKind(call) : "executing";
    return { kind, label: labels[kind], key: call?.call_id ?? generation };
  }
  // Loading, queueing, first-output waits, stopping, approvals and errors stay
  // literal. In particular, 'waiting' is not evidence of prefill progress.
  return { label: status, key: status, paused: /waiting for you|approval|failed|error|queued/i.test(status) };
}
export function ActivityStatus({ activity, seed }: { activity: ActivityDescription; seed: string }) {
  const text = activity.kind ? `${pickWorkingVerb(activity.kind, `${seed}|${activity.key}|${activity.kind}`)}…` : activity.label;
  return <div className="work-phase ag1-activity-status" role="status" aria-label={activity.label} aria-live="polite" aria-atomic="true" data-activity={activity.kind ?? "literal"} title={activity.label}>
    <ActivityMark paused={activity.paused} />
    <span className={activity.kind ? "ag1-activity-word" : undefined}>{text}</span>
  </div>;
}
