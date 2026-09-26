import { useEffect, useRef, useState, type CSSProperties } from "react";
import { Maximize2, Minimize2, X } from "lucide-react";
import type { Command, JsonValue } from "./generated/protocol";
import { ArtifactsPane } from "./ArtifactsPane";
import { WorkspaceReview } from "./WorkspaceReview";
import { TurnReview } from "./TurnChanges";
import { GitHubPullReview, type GitHubReviewTarget } from "./GitHubPullReview";
import "./review.css";

export type ReviewTab = "changes" | "artifacts" | "github";
export const boundedPanelWidth = (width: number) => Math.max(320, Math.min(960, Number.isFinite(width) ? width : 520));
export function restoreReviewFocus(opener: HTMLElement | null, tab: ReviewTab) {
  const usable = (element: HTMLElement | null): element is HTMLElement => !!element?.isConnected && !element.closest('[hidden], [inert], :disabled') && !!element.getClientRects().length;
  const fallback = document.querySelector<HTMLElement>(`button[aria-label="${tab === "changes" ? "Changes" : "Artifacts"}"]`);
  const composer = document.querySelector<HTMLElement>('textarea[aria-label="Message AG1"]');
  [opener, fallback, composer].find(usable)?.focus();
}
function savedWidth() {
  try { return boundedPanelWidth(Number(localStorage.getItem("ag1.review.width") || 520)); } catch { return 520; }
}

export function ReviewPanel({ tab, onTab, run, onClose, open = true, turn, effect, artifactTurn, github, localAvailable = true }: {
  github?: GitHubReviewTarget;
  localAvailable?: boolean;
  artifactTurn?: string;
  effect?: (command: Command) => Promise<JsonValue>;
  turn?: string;
  open?: boolean; tab: ReviewTab; onTab: (tab: ReviewTab) => void;
  run: (command: Command) => Promise<JsonValue>; onClose: () => void;
}) {
  const [width, setWidth] = useState(savedWidth), [expanded, setExpanded] = useState(false);
  const [narrow, setNarrow] = useState(() => matchMedia("(max-width: 1100px)").matches);
  const panel = useRef<HTMLElement>(null);
  const overlay = narrow || expanded;
  const close = useRef<HTMLButtonElement>(null), drag = useRef<{ x: number; width: number } | null>(null);
  useEffect(() => { if (open) close.current?.focus(); }, [open]);
  useEffect(() => {
    const media = matchMedia("(max-width: 1100px)"), update = () => setNarrow(media.matches);
    media.addEventListener?.("change", update); return () => media.removeEventListener?.("change", update);
  }, []);
  useEffect(() => {
    if (!open || !overlay) return;
    const behind = [...document.querySelectorAll<HTMLElement>(".redesigned > .main, .redesigned > .chat-sidebar")];
    const previous = behind.map(element => element.inert);
    behind.forEach(element => { element.inert = true; });
    return () => behind.forEach((element, i) => { element.inert = previous[i]; });
  }, [open, overlay]);
  useEffect(() => { try { localStorage.setItem("ag1.review.width", String(width)); } catch { /* Private storage does not prevent review. */ } }, [width]);
  return <aside ref={panel} role={overlay ? "dialog" : undefined} aria-modal={overlay ? true : undefined} hidden={!open} className={`review-panel${expanded ? " expanded" : ""}`} aria-label="Review and artifacts" style={{ "--review-width": `${width}px` } as CSSProperties} onKeyDown={e => {
    if (e.key === "Escape" && !e.defaultPrevented) { e.preventDefault(); e.stopPropagation(); onClose(); }
    if (e.key === "Tab" && overlay) {
      const controls = [...(panel.current?.querySelectorAll<HTMLElement>('button:not(:disabled), a[href], input:not(:disabled), summary, [tabindex="0"]') ?? [])].filter(element => element.getClientRects().length && !element.closest("[hidden]"));
      const first = controls[0], last = controls.at(-1);
      if (e.shiftKey && document.activeElement === first) { e.preventDefault(); last?.focus(); }
      else if (!e.shiftKey && document.activeElement === last) { e.preventDefault(); first?.focus(); }
    }
  }}>
    {!expanded && <div className="review-resize" role="separator" aria-label="Resize review panel" aria-orientation="vertical" aria-valuemin={320} aria-valuemax={960} aria-valuenow={width} tabIndex={0}
      onKeyDown={e => { const delta = e.key === "ArrowLeft" ? 32 : e.key === "ArrowRight" ? -32 : 0; if (delta || e.key === "Home" || e.key === "End") { e.preventDefault(); setWidth(old => e.key === "Home" ? 320 : e.key === "End" ? 960 : boundedPanelWidth(old + delta)); } }}
      onPointerDown={e => { if (e.button !== 0) return; drag.current = { x: e.clientX, width }; e.currentTarget.setPointerCapture(e.pointerId); e.preventDefault(); }}
      onPointerMove={e => { if (drag.current) setWidth(boundedPanelWidth(drag.current.width + drag.current.x - e.clientX)); }}
      onPointerUp={e => { drag.current = null; if (e.currentTarget.hasPointerCapture(e.pointerId)) e.currentTarget.releasePointerCapture(e.pointerId); }}
      onPointerCancel={() => { drag.current = null; }} onLostPointerCapture={() => { drag.current = null; }} />}
    <header className="review-heading"><nav aria-label="Review views">
      {localAvailable && <><button aria-pressed={tab === "changes"} onClick={() => onTab("changes")}>Review</button>
      <button aria-pressed={tab === "artifacts"} onClick={() => onTab("artifacts")}>Artifacts</button></>}
      {github && <button aria-pressed={tab === "github"} onClick={() => onTab("github")}>GitHub PR</button>}
    </nav><div><button aria-label={expanded ? "Restore panel size" : "Expand review panel"} onClick={() => setExpanded(v => !v)}>{expanded ? <Minimize2 size={16} /> : <Maximize2 size={16} />}</button><button ref={close} aria-label="Close review panel" onClick={onClose}><X size={17} /></button></div></header>
    {localAvailable && <><div className="review-body" hidden={tab !== "changes"}>{turn ? <TurnReview key={turn} turn={turn} run={run} effect={effect} /> : <WorkspaceReview run={run} visible={open && tab === "changes"} />}</div>
    <div className="review-body" hidden={tab !== "artifacts"}><ArtifactsPane key={artifactTurn ?? "conversation"} turn={artifactTurn} run={run} visible={open && tab === "artifacts"} /></div></>}
    {github && <div className="review-body" hidden={tab !== "github"}><GitHubPullReview key={`${github.repo}/${github.number}`} target={github} visible={open && tab === "github"} /></div>}
  </aside>;
}
