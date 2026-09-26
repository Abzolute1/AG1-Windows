import { useEffect, useState } from "react";
export const fonts = { system: 'Geist, system-ui, sans-serif', sans: 'Arial, sans-serif', serif: 'Georgia, serif', mono: '"Geist Mono", monospace' };
export const defaults = { uiSize: 15, chatSize: 17, codeSize: 14, statusSize: 14, uiFont: "system", chatFont: "system", codeFont: "mono", lineHeight: 1.7, chatWidth: 900, sidebarWidth: 260, density: "comfortable", motion: "system", pointers: true, sendKey: "enter", showTokens: true, showTime: true, codeWrap: true, compactTools: true, planColor: "#52b8ac", accent: "", background: "", foreground: "" };
export type Preferences = typeof defaults;
export function normalizePreferences(value: unknown): Preferences {
  const p = { ...defaults }, v = value && typeof value === "object" ? value as Record<string, unknown> : {};
  const ranges = { uiSize: [13, 22], chatSize: [13, 28], codeSize: [11, 24], statusSize: [12, 20], lineHeight: [1.35, 2], chatWidth: [640, 1200], sidebarWidth: [200, 360] };
  for (const [key, [lo, hi]] of Object.entries(ranges)) { const n = Number(v[key]); if (Number.isFinite(n) && v[key] !== null && v[key] !== undefined) (p as any)[key] = Math.max(lo, Math.min(hi, n)); }
  for (const key of ["uiFont", "chatFont", "codeFont"] as const) if (typeof v[key] === "string" && Object.hasOwn(fonts, v[key])) p[key] = v[key];
  for (const key of ["pointers", "showTokens", "showTime", "codeWrap", "compactTools"] as const) if (typeof v[key] === "boolean") p[key] = v[key];
  for (const key of ["accent", "background", "foreground", "planColor"] as const) if (typeof v[key] === "string" && /^#[a-f\d]{6}$/i.test(v[key])) p[key] = v[key];
  for (const [key, choices] of Object.entries({ density: ["comfortable", "compact"], motion: ["system", "on", "off"], sendKey: ["enter", "modifier"] })) if (choices.includes(v[key] as string)) (p as any)[key] = v[key];
  return p;
}
export function usePreferences() {
  const [preferences, setPreferences] = useState<Preferences>(() => { try { const saved = localStorage?.getItem("ag1.appearance.v2"); return normalizePreferences(saved ? JSON.parse(saved) : { chatSize: Number(localStorage?.getItem("ag1.font-size")) || defaults.chatSize }); } catch { return { ...defaults }; } });
  useEffect(() => { localStorage?.setItem("ag1.appearance.v2", JSON.stringify(preferences)); applyPreferences(preferences); }, [preferences]);
  return [preferences, (patch: Partial<Preferences>) => setPreferences(p => normalizePreferences({ ...p, ...patch }))] as const;
}
export function applyPreferences(p: Preferences) {
  const root = document.documentElement, s = root.style;
  s.setProperty("--plan-color", p.planColor);
  for (const [key, value] of Object.entries({ "ui-font-size": `${p.uiSize}px`, "chat-font-size": `${p.chatSize}px`, "code-font-size": `${p.codeSize}px`, "status-font-size": `${p.statusSize}px`, "chat-line-height": String(p.lineHeight), "chat-width": `${p.chatWidth}px`, "sidebar-width": `${p.sidebarWidth}px`, "ui-font": fonts[p.uiFont as keyof typeof fonts], "chat-font": fonts[p.chatFont as keyof typeof fonts], "code-font": fonts[p.codeFont as keyof typeof fonts] })) s.setProperty(`--${key}`, value);
  for (const [key, value] of Object.entries({ primary: p.accent, ring: p.accent, link: p.accent, background: p.background, foreground: p.foreground })) { if (value) s.setProperty(`--${key}`, value); else s.setProperty(`--${key}`, `var(--theme-${key})`); }
  if (p.accent) { const rgb = p.accent.slice(1).match(/../g)!.map(n => parseInt(n, 16)); s.setProperty("--primary-foreground", rgb[0] * .299 + rgb[1] * .587 + rgb[2] * .114 > 150 ? "#151518" : "#ffffff"); } else s.setProperty("--primary-foreground", "var(--theme-primary-foreground)");
  root.dataset.density = p.density; root.dataset.motion = p.motion; root.dataset.pointers = String(p.pointers); root.dataset.codeWrap = String(p.codeWrap);
}
