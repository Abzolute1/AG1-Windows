import builtins from "./builtin-themes.json";
export type Theme = string;
export type ThemePlugin = { schemaVersion: number; id: string; name: string; mode: string; colors: Record<string, string> };
export const builtinThemes: ThemePlugin[] = builtins;
export const themeNames: Record<string, string> = Object.fromEntries([...builtins.map(t => [t.id, t.name]), ["system", "System"]]);
export function resolveTheme(id: string, plugins: ThemePlugin[] = []) {
  const selected = id === "system" ? (matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light") : id;
  return [...builtinThemes, ...plugins].find(t => t.id === selected) ?? builtinThemes[0];
}
export function applyTheme(theme: Theme, fontSize: number, plugins: ThemePlugin[] = []) {
  const selected = resolveTheme(theme, plugins), root = document.documentElement;
  root.dataset.theme = selected.id;
  root.classList.toggle("dark", selected.mode !== "light");
  for (const [key, fallback] of Object.entries(builtinThemes[0].colors)) {
    const color = selected.colors[key];
    root.style.setProperty(`--theme-${key}`, /^#[a-f\d]{6}$/i.test(color) ? color : fallback);
    root.style.setProperty(`--${key}`, `var(--theme-${key})`);
  }
  root.style.setProperty("--chat-font-size", `${Math.max(13, Math.min(28, fontSize))}px`);
  root.style.colorScheme = selected.mode === "light" ? "light" : "dark";
}
