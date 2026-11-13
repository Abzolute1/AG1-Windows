import { useEffect, useRef, useState } from "react";
import { Globe, LoaderCircle, Search, Terminal } from "lucide-react";
import { ChatBlocks } from "./ChatBlocks";
import type { Live } from "./NativeConversation";
import type { Command, JsonValue, MessageBlock } from "./generated/protocol";
type StreamBlock = MessageBlock & { key: string; revision?: number; bytes?: number; truncated?: boolean; limited?: boolean };
type ToolBlock = Extract<MessageBlock, { kind: "tool" }>;
type StreamToolBlock = ToolBlock & Pick<StreamBlock, "key" | "revision" | "bytes" | "truncated" | "limited">;

// Tool arguments can be incomplete while a local model is still producing
// them. Show the useful state, not a misleading empty JSON object.
export function liveToolDisplay(tool: ToolBlock): { label: string; target: string; kind: "search" | "page" | "tool" } {
  const args = tool.arguments && typeof tool.arguments === "object" && !Array.isArray(tool.arguments)
    ? tool.arguments as Record<string, unknown>
    : {};
  if (tool.name === "webSearch") {
    const query = typeof args.query === "string" ? args.query.trim().slice(0, 240) : "";
    return { label: query ? "Searching the web" : "Preparing web search", target: query, kind: "search" };
  }
  if (tool.name === "webFetch") {
    const url = typeof args.url === "string" ? args.url.trim().slice(0, 240) : "";
    return { label: "Reading webpage", target: url, kind: "page" };
  }
  if (tool.name === "mcpSearch") {
    const query = typeof args.query === "string" ? args.query.trim().slice(0, 240) : "";
    return { label: query ? "Searching connected tools" : "Preparing tool search", target: query, kind: "search" };
  }
  if (tool.name === "mcpCall") {
    const name = typeof args.name === "string" ? args.name.trim().slice(0, 240) : "";
    return { label: name ? "Running connected tool" : "Preparing connected tool", target: name, kind: "tool" };
  }
  return { label: `Using ${tool.name}`, target: "", kind: "tool" };
}

function LiveTool({ tool }: { tool: ToolBlock }) {
  const display = liveToolDisplay(tool);
  const Icon = display.kind === "search" ? Search : display.kind === "page" ? Globe : Terminal;
  return (
    <div className="live-tool" role="status" aria-label={display.target ? `${display.label}: ${display.target}` : display.label}>
      <LoaderCircle size={14} className="spin" aria-hidden="true" />
      <Icon size={14} aria-hidden="true" />
      <span>{display.label}</span>
      {display.target && <small title={display.target}>{display.target}</small>}
    </div>
  );
}

/** Small streams ride the view. Long streams page independently without freezing the answer behind thinking. */
export function LiveBlocks({ live, run, thinking, onThinking }: { live: Live; run: (command: Command) => Promise<JsonValue>; thinking: Record<number, boolean>; onThinking: (index: number, open: boolean) => void }) {
  const latest = useRef(live); latest.current = live;
  const [complete, setComplete] = useState<Record<string, string>>({});
  useEffect(() => {
    let closed = false, timer: ReturnType<typeof setTimeout>;
    const cursors = new Map<string, { offset: number; text: string; decoder: TextDecoder }>();
    async function poll() {
      try {
        for (const block of latest.current.blocks as StreamBlock[]) {
          if (!block.truncated || !(block.kind === "thinking" || block.kind === "text")) continue;
          const identity = `${block.key}:${block.revision ?? 0}`;
          let cursor = cursors.get(identity);
          if (!cursor) { cursor = { offset: 0, text: "", decoder: new TextDecoder() }; cursors.set(identity, cursor); }
          if (cursor.offset >= (block.bytes ?? 0)) continue;
          const page = await run({ kind: "runtime", action: "live_block", args: { id: live.id, key: block.key, revision: block.revision ?? 0, offset: cursor.offset } }) as { base64: string; offset: number; total: number };
          if (closed) return;
          cursor.text += cursor.decoder.decode(Uint8Array.from(atob(page.base64), c => c.charCodeAt(0)), { stream: true }); cursor.offset = page.offset;
          const text = cursor.text; setComplete(old => ({ ...old, [identity]: text }));
        }
      } catch { /* A new generation or a committed journal supersedes this live page. */ }
      finally { if (!closed) timer = setTimeout(poll, 50); }
    }
    void poll(); return () => { closed = true; clearTimeout(timer); };
  }, [live.id, run]);
  const blocks = (live.blocks as StreamBlock[]).map(b => (b.kind === "text" || b.kind === "thinking") && (complete[`${b.key}:${b.revision ?? 0}`]?.length ?? 0) > b.text.length ? { ...b, text: complete[`${b.key}:${b.revision ?? 0}`] } : b);
  const tools = blocks.filter((block): block is StreamToolBlock => block.kind === "tool");
  // Committed calls use ToolActivity, which owns source cards and favicons.
  // This row only represents work that has not produced a result yet.
  const visible = blocks.filter(block => block.kind !== "tool");
  return <><ChatBlocks run={run} blocks={visible} live thinking={thinking} onThinking={onThinking} />{tools.map(tool => <LiveTool key={tool.call_id} tool={tool} />)}{(live.blocks as StreamBlock[]).some(b => b.limited) && <p className="muted">Live display reached 4 MiB. The complete message will be available from its journal after generation.</p>}</>;
}
