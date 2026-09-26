import { useState } from "react";
import "./responseStatus.css";
import { ChevronRight, LoaderCircle, Terminal } from "lucide-react";
import type { MessageBlock } from "./generated/protocol";
import { JournalImages, type ImageRun } from "./ImageAttachment";
import { MCPResult } from "./MCPResult";
import { Markdown } from "./Markdown";
export function Thinking({ text, live = false, expanded, onToggle }: { text: string; live?: boolean; expanded?: boolean; onToggle?: (open: boolean) => void }) {
  const [localOpen, setOpen] = useState(false);
  const open = expanded ?? localOpen;
  if (!live && !text.trim()) return null;
  return <div className="thinking"><button className="disclosure" aria-expanded={open} onClick={() => { setOpen(!open); onToggle?.(!open); }}><ChevronRight size={14} className={open ? "expanded" : ""} />{live && <LoaderCircle size={13} className="spin" />}{live ? "Thinking" : "Thoughts"}</button>{open && <div className="thinking-content" aria-label={live ? "Live thinking" : "Model thinking"}><Markdown text={text} /></div>}</div>;
}
export function ChatBlocks({ blocks, live = false, thinking = {}, onThinking, run }: { run?: ImageRun; blocks: MessageBlock[]; live?: boolean; thinking?: Record<number, boolean>; onThinking?: (index: number, open: boolean) => void }) {
  return <>{blocks.map((b, i) => b.kind === "thinking" ? <Thinking key={i} text={b.text} live={live && !blocks.slice(i + 1).some(next => next.kind === "tool" || next.kind === "text" && next.text.trim())} expanded={thinking[i]} onToggle={open => onThinking?.(i, open)} /> : b.kind === "text" ? <Markdown key={i} text={b.text} /> : b.kind === "tool" ? <details className="tool-call" key={i}><summary><Terminal size={14} />{b.name}</summary><pre>{JSON.stringify(b.arguments, null, 2)}</pre></details> : b.kind === "attachment" ? run && b.media_type.startsWith("image/") ? blocks.slice(0, i).some(previous => previous.kind === "attachment" && previous.reference === b.reference) ? null : <JournalImages key={b.reference} reference={b.reference} run={run} /> : <span className="attachment-chip" key={i}>Attachment · {b.media_type}</span> : b.value && typeof b.value === "object" && !Array.isArray(b.value) && b.value.kind === "web_result" ? null : b.value && typeof b.value === "object" && !Array.isArray(b.value) && b.value.kind === "mcp_result" && run ? <MCPResult key={i} value={b.value} run={run} /> : b.value && typeof b.value === "object" && !Array.isArray(b.value) && b.value.kind === "provider_error" ? <div key={i} className="provider-error" role="alert"><strong>{b.value.cancelled ? "Response stopped" : "Response failed"}</strong><p>{String(b.value.message ?? "The response ended before completion.")}</p><p className="muted">This response is no longer running. You can send a new message to continue.</p></div> : <details key={i}><summary>Preserved content</summary><pre>{JSON.stringify(b.value)}</pre></details>)}</>;
}
