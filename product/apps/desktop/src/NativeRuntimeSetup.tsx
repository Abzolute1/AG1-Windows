import { useState } from "react";
import type { NativeModelSetup } from "./bridge";

type Kind = "server" | "model" | "projector" | "profile";

export function NativeRuntimeSetup({ configure, chooseFile }: {
  configure: (setup: NativeModelSetup) => Promise<void>;
  chooseFile: (kind: Kind) => Promise<string | null>;
}) {
  const [modelId, setModelId] = useState("");
  const [serverPath, setServerPath] = useState("");
  const [modelPath, setModelPath] = useState("");
  const [projectorPath, setProjectorPath] = useState("");
  const [profilePath, setProfilePath] = useState("");
  const [port, setPort] = useState("18082");
  const [vram, setVram] = useState("");
  const [systemPrompt, setSystemPrompt] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const browse = async (kind: Kind, set: (value: string) => void) => {
    setError("");
    const path = await chooseFile(kind);
    if (path) set(path);
  };
  const save = async () => {
    const selectedPort = Number(port);
    const vramMiB = vram.trim() ? Number(vram) : undefined;
    if (!modelId.trim() || !serverPath || !modelPath || !profilePath || !Number.isInteger(selectedPort) || selectedPort < 1 || selectedPort > 65535 || vramMiB !== undefined && (!Number.isInteger(vramMiB) || vramMiB < 512)) {
      setError("Choose all three files, a model ID, and a valid local port. VRAM is optional and measured in MiB.");
      return;
    }
    setBusy(true);
    setError("");
    try {
      await configure({ modelId: modelId.trim(), serverPath, modelPath, profilePath, projectorPath: projectorPath.trim() || undefined, port: selectedPort, vramMiB, systemPrompt });
    } catch (value) {
      setError(value instanceof Error ? value.message : String(value));
    } finally {
      setBusy(false);
    }
  };
  const picker = (label: string, kind: Kind, value: string, set: (value: string) => void, placeholder: string) => <label className="native-runtime-path"><span>{label}</span><div><input value={value} placeholder={placeholder} onChange={event => set(event.target.value)} /><button type="button" className="outline-button" disabled={busy} onClick={() => void browse(kind, set)}>Browse</button></div></label>;
  return <section className="settings-card native-runtime-setup" aria-label="Windows local runtime setup">
    <h3>Windows local runtime</h3>
    <p>Choose the native llama-server executable, one GGUF, and the matching AG1 tuning profile. AG1W automatically uses one matching mmproj GGUF beside the model. Choose a vision projector only when it lives elsewhere or the folder has several. Saving hashes and seals these files; it does not load the model.</p>
    <label>Model ID<input value={modelId} maxLength={128} placeholder="qwen-local" onChange={event => setModelId(event.target.value)} /></label>
    {picker("llama-server.exe", "server", serverPath, setServerPath, "C:\\Tools\\llama-server.exe")}
    {picker("GGUF model", "model", modelPath, setModelPath, "C:\\Users\\you\\Documents\\Models\\model.gguf")}
    {picker("Vision projector · optional", "projector", projectorPath, setProjectorPath, "C:\\Users\\you\\Documents\\Models\\mmproj-model.gguf")}
    {picker("AG1 tuning profile", "profile", profilePath, setProfilePath, "C:\\Users\\you\\Documents\\Models\\profile.json")}
    <div className="native-runtime-numbers"><label>Local port<input inputMode="numeric" value={port} onChange={event => setPort(event.target.value)} /></label><label>VRAM MiB <small>optional</small><input inputMode="numeric" value={vram} onChange={event => setVram(event.target.value)} /></label></div>
    <label>System prompt <small>optional</small><textarea rows={3} value={systemPrompt} onChange={event => setSystemPrompt(event.target.value)} /></label>
    {error && <p className="settings-error" role="alert">{error}</p>}
    <button type="button" className="primary-button" disabled={busy} onClick={() => void save()}>{busy ? "Verifying model files…" : "Verify and save Windows runtime"}</button>
  </section>;
}
