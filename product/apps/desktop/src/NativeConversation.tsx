import { memo, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { ArrowDown, Copy, ChevronRight, LoaderCircle } from "lucide-react";
import { ActivityStatus, describeActivity } from "./ActivityStatus";
import { LiveBlocks } from "./LiveBlocks";
import { ChatBlocks } from "./ChatBlocks";
import { ActivityGroup } from "./ToolActivity";
import { TurnChangeCard } from "./TurnChanges";
// TurnOutputs is temporarily hidden from the chat footer; Artifacts remains available.
import type { BridgeSnapshot, ClientDetails, ClientMessage, Command, JsonValue, MessageBlock } from "./generated/protocol";
type Run = (command: Command) => Promise<JsonValue>;
type Page = { messages: ClientMessage[]; next_before: number | null; start: number; total: number };
export type Live = { id: string; blocks: MessageBlock[]; committed: boolean };
type Turn = { id: string; messages: ClientMessage[] };
export function TurnView({ turn, live, active, run, compactTools = true, reviewChanges, reviewOutputs, workTime, workStatus }: { turn: Turn; live?: Live; active: boolean; run: Run; compactTools?: boolean; workTime?: string; workStatus?: string; reviewChanges?: (turn: string) => void; reviewOutputs?: (turn: string) => void }) {
  const [requested, setRequested] = useState<Set<string>>(new Set());
  const expand = (id: string) => setRequested(old => old.has(id) ? old : new Set([...old, id]));
  const [full, setFull] = useState<Record<string, ClientMessage>>({}), [error, setError] = useState("");
  const fetching = useRef(new Set<string>());
  const [thinking, setThinking] = useState<Record<string, Record<number, boolean>>>({});
  const mounted = useRef(true);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  // Projection pages stay bounded; complete display is loaded from the durable journal.
  useEffect(() => {
    for (const m of turn.messages) {
      if (m.role === "toolResult" && !requested.has(m.id)) continue;
      if (fetching.current.has(m.id) || !m.blocks.some(b => (b.kind === "text" || b.kind === "thinking") && b.text.includes("[open journal page]") || b.kind === "tool" && JSON.stringify(b.arguments).includes("Open journal page"))) continue;
      fetching.current.add(m.id);
      void (async () => {
        let offset: string | null = "0", text = ""; const decoder = new TextDecoder();
        while (offset !== null) {
          const page = await run({ kind: "session", action: "entry", args: { id: m.id, offset } }) as { base64: string; next: string | null };
          text += decoder.decode(Uint8Array.from(atob(page.base64), c => c.charCodeAt(0)), { stream: page.next !== null }); offset = page.next;
          if (text.length > 4 * 1024 * 1024) throw new Error("Message exceeds the display limit; use journal export for its complete contents.");
        }
        const entry = JSON.parse(text), content = entry.message.content;
        const blocks: MessageBlock[] = typeof content === "string" ? [{ kind: "text", text: content }] : content.map((b: any): MessageBlock => b.type === "text" ? { kind: "text", text: b.text } : b.type === "thinking" ? { kind: "thinking", text: b.thinking } : b.type === "toolCall" ? { kind: "tool", name: b.name, call_id: b.id, arguments: b.arguments } : b.type === "image" ? { kind: "attachment", media_type: b.mimeType, reference: m.id } : { kind: "unknown", value: b });
        blocks.unshift(...m.blocks.filter(b => b.kind === "unknown" && b.value && typeof b.value === "object" && !Array.isArray(b.value) && ["mcp_result", "web_result"].includes(String(b.value.kind))));
        if (mounted.current) setFull(old => ({ ...old, [m.id]: { ...m, blocks } }));
      })().catch(e => { if (mounted.current) setError(String(e)); });
    }
  }, [turn.messages, run, requested]);
  const messages = turn.messages.map(m => full[m.id] ?? m);
  const user = messages.find(m => m.role === "user"), rest = messages.filter(m => m.role !== "user");
  const last = rest.at(-1);
  const terminalFailure = last?.blocks.some(b => b.kind === "unknown" && b.value && typeof b.value === "object" && !Array.isArray(b.value) && b.value.kind === "provider_error");
  if (terminalFailure) active = false;
  const final = (!active || live?.committed) && last?.role === "assistant" && !last.blocks.some(b => b.kind === "tool") ? last : undefined;
  const work = final ? rest.slice(0, -1) : rest;
  const thoughts = (index: number) => ({ thinking: thinking[index] ?? {}, onThinking: (block: number, open: boolean) => setThinking(old => ({ ...old, [index]: { ...old[index], [block]: open } })) });
  const assistantIndex = (m: ClientMessage) => rest.filter(r => r.role === "assistant").findIndex(r => r.id === m.id);
  const renderAssistant = (assistant: ClientMessage) => <>
    <div className="assistant-message progress-commentary"><ChatBlocks run={run} blocks={assistant.blocks.filter(b => b.kind === "text" || b.kind === "attachment")} /></div>
    {assistant.blocks.map((b, i) => b.kind === "thinking" ? <ChatBlocks key={i} blocks={[b]} thinking={{ 0: thinking[assistantIndex(assistant)]?.[i] ?? false }} onThinking={(_, open) => thoughts(assistantIndex(assistant)).onThinking(i, open)} /> : null)}
  </>;
  return <article className="chat-turn" data-turn-id={turn.id}>
    {user && <div className="user-message"><ChatBlocks run={run} blocks={user.blocks} /></div>}
    {(active || !!work.length) && <ActivityGroup messages={work} active={active} time={workTime} compact={compactTools} run={run} onExpand={expand} renderAssistant={renderAssistant} loading={id => requested.has(id) && !full[id] && !!turn.messages.find(m => m.id === id)?.blocks.some(b => b.kind === "text" && b.text.includes("[open journal page]"))} />}
    {final && <div className="assistant-message final-answer"><ChatBlocks run={run} blocks={final.blocks} {...thoughts(assistantIndex(final))} /><button className="copy-answer" aria-label="Copy answer" onClick={() => void navigator.clipboard.writeText(final.blocks.filter(b => b.kind === "text").map(b => b.text).join("\n")).catch(e => setError(String(e)))}><Copy size={14} /></button></div>}
    {user && reviewChanges && messages.some(m => m.blocks.some(b => b.kind === "tool" && ["write", "edit"].includes(b.name))) && <TurnChangeCard turn={user.id} revision={`${active}/${messages.map(m => m.id).join("/")}`} run={run} review={reviewChanges} />}
    {/* Retained-output footer temporarily hidden at user request. */}
    {error && <p role="alert">{error}</p>}
    {active && live && !live.committed && <div className="assistant-message live-answer" key={live.id}><LiveBlocks live={live} run={run} {...thoughts(rest.filter(m => m.role === "assistant").length)} /></div>}
    {active && workStatus && <ActivityStatus activity={describeActivity(workStatus, messages, live)} seed={turn.id} />}
  </article>;
}
const MemoTurn = memo(TurnView);
export function mergeMessages(previous: ClientMessage[], incoming: ClientMessage[], prepend = false): ClientMessage[] {
  const map = new Map((prepend ? incoming : previous).map(m => [m.id, m]));
  for (const m of prepend ? previous : incoming) { const old = map.get(m.id); map.set(m.id, old && JSON.stringify(old) === JSON.stringify(m) ? old : m); }
  return [...map.values()];
}
export function ImageAnalysisStatus({ details, snapshot }: { details: ClientDetails; snapshot: BridgeSnapshot }) {
  const activity = details.activity as any, observation = activity?.observations, preparation = activity?.runtime_preparation;
  const user = details.messages.filter(m => m.role === "user").at(-1);
  const waiting = ["streaming", "running"].includes(snapshot.status) && !snapshot.pending.length
    && observation?.first_delta_ms == null && !["thinking", "generating", "tool", "provider stopped"].includes(observation?.phase)
    && !["verifying", "loading", "failed"].includes(preparation?.phase)
    && user?.blocks.some(b => b.kind === "attachment" && b.media_type.startsWith("image/"));
  return waiting ? <div className="chat-turn image-analysis-status" role="status"><LoaderCircle size={15} className="spin" />Analyzing image</div> : null;
}
export const NativeConversation = memo(function NativeConversation({ details, snapshot, run, compactTools = true, reviewChanges, reviewOutputs, workTime, workStatus }: { details: ClientDetails; snapshot: BridgeSnapshot; run: Run; compactTools?: boolean; workTime?: string; workStatus?: string; reviewChanges?: (turn: string) => void; reviewOutputs?: (turn: string) => void }) {
  const [history, setHistory] = useState<Map<number, ClientMessage>>(new Map()), [busy, setBusy] = useState(false), [error, setError] = useState(""), [bottom, setBottom] = useState(true);
  const scroll = useRef<HTMLDivElement>(null), contents = useRef<HTMLDivElement>(null), following = useRef(true), loading = useRef(false), mounted = useRef(true);
  const anchor = useRef<{ id: string; top: number } | null>(null);
  const indexed = useMemo(() => {
    const next = new Map(history), start = Math.max(0, snapshot.message_count - details.messages.length);
    details.messages.forEach((m, i) => { const old = next.get(start + i); next.set(start + i, old && JSON.stringify(old) === JSON.stringify(m) ? old : m); });
    return new Map([...next].sort(([a], [b]) => a - b));
  }, [history, details.messages, snapshot.message_count]);
  const messages = useMemo(() => [...indexed.values()], [indexed]);
  // The host's bounded view slides forward. Retain every loaded message locally.
  useEffect(() => { setHistory(old => old.size === indexed.size && [...indexed].every(([i, m]) => m === old.get(i)) ? old : indexed); }, [indexed]);
  const turnCache = useRef(new Map<string, Turn>());
  const turns = useMemo(() => { const result: Turn[] = []; for (const m of messages) { if (m.role === "user" || !result.length) result.push({ id: m.id, messages: [] }); result.at(-1)!.messages.push(m); } return result.map(t => { const old = turnCache.current.get(t.id); if (old && old.messages.length === t.messages.length && old.messages.every((m, i) => m === t.messages[i])) return old; turnCache.current.set(t.id, t); return t; }); }, [messages]);
  const observations = (details.activity as any)?.observations, live = observations?.live as Live | undefined;
  const active = ["streaming", "running", "awaiting_input"].includes(snapshot.status);
  const positions = [...indexed.keys()];
  const remaining = positions[0] ?? snapshot.message_count;
  const gapEnd = positions.find((n, i) => i > 0 && n > positions[i - 1] + 1);
  const earlierRef = useRef<() => void>(() => {});
  async function earlier(before = remaining) {
    if (loading.current || before <= 0) return;
    loading.current = true; setBusy(true); setError("");
    try {
      const page = await run({ kind: "session", action: "messages", args: { before } }) as unknown as Page;
      if (!mounted.current) return;
      if (!page.messages.length || page.next_before !== null && page.next_before >= before) throw new Error("History could not advance. Retry loading earlier messages.");
      const parent = scroll.current, first = parent && [...parent.querySelectorAll<HTMLElement>("[data-turn-id]")].find(e => e.getBoundingClientRect().bottom > parent.getBoundingClientRect().top);
      if (first && !following.current) anchor.current = { id: first.dataset.turnId!, top: first.getBoundingClientRect().top };
      setHistory(old => { const next = new Map(old); page.messages.forEach((m, i) => next.set(page.start + i, m)); return next; });
    } catch (e) { if (mounted.current) setError(String(e)); } finally { loading.current = false; if (mounted.current) setBusy(false); }
  }
  earlierRef.current = () => void earlier(gapEnd ?? remaining);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  useLayoutEffect(() => {
    const el = scroll.current; if (!el) return;
    if (anchor.current) { const a = anchor.current; const row = [...el.querySelectorAll<HTMLElement>("[data-turn-id]")].find(e => e.dataset.turnId === a.id); if (row) el.scrollTop += row.getBoundingClientRect().top - a.top; anchor.current = null; }
    else if (following.current) el.scrollTop = el.scrollHeight;
  }, [messages]);
  useEffect(() => {
    const el = scroll.current, content = contents.current; if (!el || !content) return;
    let frame = 0;
    const observer = new ResizeObserver(() => { cancelAnimationFrame(frame); frame = requestAnimationFrame(() => { if (following.current) el.scrollTop = el.scrollHeight; setBottom(el.scrollHeight - el.scrollTop - el.clientHeight < 60); }); }); observer.observe(content);
    return () => { observer.disconnect(); cancelAnimationFrame(frame); };
  }, []);
  // Recover the start of a partially visible turn; also fill short initial views.
  useEffect(() => { if (!busy && !error && (remaining > 0 || gapEnd !== undefined) && (gapEnd !== undefined || messages[0]?.role !== "user" || (scroll.current && scroll.current.scrollHeight <= scroll.current.clientHeight + 10))) earlierRef.current(); }, [remaining, gapEnd, busy, error, messages]);
  return <section className="native-conversation" aria-label="Conversation"><div className="conversation-scroll" ref={scroll} tabIndex={0} aria-label="Chat history" onClickCapture={event => { if ((event.target as Element).closest(".work-toggle, .tool-entry-toggle, .output-more, .tool-arguments-toggle")) following.current = false; }} onScroll={() => { const el = scroll.current!; const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 60; following.current = atBottom; setBottom(atBottom); if (el.scrollTop < 90 && remaining > 0 && !error) earlierRef.current(); }}><div ref={contents} className="conversation-content">
    {remaining > 0 && <button className="history-controls" disabled={busy} onClick={() => { following.current = false; void earlier(); }}>{busy ? "Loading earlier messages…" : "Load earlier messages"}</button>}
    {error && <div className="history-error" role="alert">{error}<button onClick={() => void earlier()}>Retry</button></div>}
    {turns.length ? turns.map((t, i) => <MemoTurn key={t.id} turn={t} run={run} workTime={i === turns.length - 1 ? workTime : undefined} workStatus={i === turns.length - 1 ? workStatus : undefined} compactTools={compactTools} reviewChanges={reviewChanges} reviewOutputs={reviewOutputs} active={active && i === turns.length - 1} live={i === turns.length - 1 ? live : undefined} />) : <div className="chat-welcome"><h1>What’s on your mind?</h1><p>Ask a question, make a plan, or attach a project.</p></div>}
    <ImageAnalysisStatus details={details} snapshot={snapshot} />
    </div></div>{!bottom && <button className="jump-latest" aria-label="Jump to latest message" onClick={() => { following.current = true; setBottom(true); scroll.current?.scrollTo({ top: scroll.current.scrollHeight, behavior: "auto" }); }}><ArrowDown size={17} /></button>}
  </section>;
});
