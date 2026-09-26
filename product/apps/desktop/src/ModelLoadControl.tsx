import { useEffect, useState } from "react";
import { Check, Download, LoaderCircle } from "lucide-react";
import type { Model } from "./bridge";
import "./modelLoading.css";
export type ModelPreparation = { model: string; phase: string; percent?: number; started_at?: number; error?: string };
export type PendingModelLoad = { model: string; started_at: number };
const labels: Record<string, string> = { verifying: "Checking model files", releasing: "Releasing previous model", loading: "Loading model", initializing: "Initializing model", warming: "Warming up", queued: "Waiting for model slot" };
export function modelPreparing(value?: ModelPreparation | null) { return !!value && value.phase in labels; }
export function ModelLoadControl({ model, progress, pending, disabled, load }: { model?: Model; progress?: ModelPreparation | null; pending: PendingModelLoad | null; disabled: boolean; load: () => void }) {
  const [now, setNow] = useState(Date.now);
  const active = modelPreparing(progress);
  const waiting = !!pending && (!progress || progress.model !== pending.model || (progress.started_at ?? 0) < pending.started_at);
  const phase = waiting ? "queued" : active ? progress!.phase : undefined;
  const started = waiting ? pending!.started_at : progress?.started_at;
  useEffect(() => { if (!phase) return; setNow(Date.now()); const timer = setInterval(() => setNow(Date.now()), 1000); return () => clearInterval(timer); }, [phase, started]);
  if (model?.provider !== "llamacpp" && !phase) return null;
  const selectedReady = progress?.phase === "ready" && progress.model === model?.id && !pending;
  const percent = !waiting && phase && typeof progress?.percent === "number" && Number.isFinite(progress.percent) ? Math.max(0, Math.min(100, Math.floor(progress.percent))) : undefined;
  const label = phase ? labels[phase] : selectedReady ? "Loaded" : progress?.phase === "failed" && progress.model === model?.id ? "Load failed" : "";
  const seconds = started ? Math.max(0, Math.floor((now - started) / 1000)) : 0;
  const elapsed = seconds < 60 ? `${seconds}s` : `${Math.floor(seconds / 60)}m ${seconds % 60}s`;
  const switchModel = progress?.phase === "ready" && progress.model !== model?.id;
  return <div className="model-load-control">
    {(phase || label) && <span className="model-load-state" title={phase ? (waiting ? pending!.model : progress?.model) : progress?.error || model?.id}>
      {phase ? percent !== undefined ? <svg className="model-load-ring" viewBox="0 0 24 24" role="progressbar" aria-label={label} aria-valuemin={0} aria-valuemax={100} aria-valuenow={percent}><circle className="model-load-track" cx="12" cy="12" r="9" /><circle cx="12" cy="12" r="9" pathLength="100" strokeDasharray={`${percent} 100`} transform="rotate(-90 12 12)" /></svg> : <LoaderCircle size={16} className="model-load-spinner" aria-hidden="true" /> : selectedReady ? <Check size={14} aria-hidden="true" /> : null}
      <span role="status">{label}{percent !== undefined ? ` · ${percent}%` : ""}</span>
      {phase && started && <span className="model-load-elapsed" aria-hidden="true">{elapsed}</span>}
    </span>}
    {!phase && !selectedReady && model?.provider === "llamacpp" && <button type="button" className="model-load-button" disabled={disabled || !!pending || !!model.conflict} onClick={load} title="Load the selected model without sending a message"><Download size={14} aria-hidden="true" />{switchModel ? "Switch & load" : "Load"}</button>}
  </div>;
}
