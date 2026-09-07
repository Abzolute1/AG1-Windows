import { useEffect, useState } from "react";
import { Button } from "./components/ui/button";
import type { Command, JsonValue } from "./generated/protocol";
type Run = (command: Command) => Promise<JsonValue>;
type Settings = { flags: Record<string, JsonValue>; shortcuts: { shortcuts: { key: string; description?: string }[]; diagnostics: JsonValue[] } };
export function ExtensionShortcuts({ run }: { run: Run }) {
  useEffect(() => {
    let closed = false; const keys = new Set<string>();
    void run({ kind: "runtime", action: "extension_settings", args: {} }).then(value => { if (!closed) for (const item of (value as unknown as Settings).shortcuts.shortcuts) keys.add(item.key.toLowerCase()); }).catch(() => {});
    const key = (e: KeyboardEvent) => {
      if (e.defaultPrevented || e.isComposing || !(e.ctrlKey || e.altKey || e.metaKey || /^F\d+$/.test(e.key))) return;
      const value = [e.ctrlKey ? "ctrl" : "", e.altKey ? "alt" : "", e.shiftKey ? "shift" : "", e.metaKey ? "meta" : "", e.key.toLowerCase()].filter(Boolean).join("+");
      if (!keys.has(value)) return;
      e.preventDefault(); void run({ kind: "runtime", action: "extension_shortcut", args: { key: value } }).catch(() => {});
    };
    document.addEventListener("keydown", key); return () => { closed = true; document.removeEventListener("keydown", key); };
  }, [run]);
  return null;
}
export function ExtensionSettings({ run }: { run: Run }) {
  const [flags, setFlags] = useState<{ name: string; type?: string; description?: string }[]>([]), [settings, setSettings] = useState<Settings | null>(null), [error, setError] = useState("");
  async function refresh() {
    try {
      setError(""); const catalog = await run({ kind: "runtime", action: "extension_catalog", args: {} }) as { flags?: typeof flags; status?: string }; setFlags(catalog.flags ?? []); if (catalog.status) { setSettings(null); return; }
      setSettings(await run({ kind: "runtime", action: "extension_settings", args: {} }) as unknown as Settings);
    } catch (e) { setError(String(e)); }
  }
  useEffect(() => { void refresh(); }, [run]);
  function execute(action: string, args: JsonValue) { void run({ kind: "runtime", action, args }).catch(e => setError(String(e))); }
  return <section><h3>Extension flags and shortcuts</h3>{error && <p role="alert">{error}</p>}<Button onClick={() => void refresh()}>Refresh extension settings</Button>{flags.map(flag => <label key={flag.name} title={flag.description}>{flag.name}{flag.type === "boolean" ? <input type="checkbox" checked={settings?.flags[flag.name] === true} onChange={e => { const value = e.target.checked; setSettings(current => current && { ...current, flags: { ...current.flags, [flag.name]: value } }); execute("extension_flag", { name: flag.name, value }); }} /> : <input value={String(settings?.flags[flag.name] ?? "")} onChange={e => { const value = e.target.value; setSettings(current => current && { ...current, flags: { ...current.flags, [flag.name]: value } }); }} onBlur={e => execute("extension_flag", { name: flag.name, value: e.target.value })} />}</label>)}<p>Flags apply to the admitted helper for this session. Refresh reads acknowledged values; a new helper restores extension defaults.</p>{settings?.shortcuts.shortcuts.map(shortcut => <Button key={shortcut.key} title={shortcut.description} onClick={() => execute("extension_shortcut", { key: shortcut.key })}>{shortcut.key}</Button>)}{!!settings?.shortcuts.diagnostics.length && <pre>{JSON.stringify(settings.shortcuts.diagnostics, null, 2)}</pre>}</section>;
}
