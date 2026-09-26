import { useEffect, useRef } from "react";
import * as monaco from "monaco-editor/esm/vs/editor/editor.api";
import { fonts } from "./preferences";
import type { Theme } from "./themes";
import EditorWorker from "monaco-editor/esm/vs/editor/editor.worker?worker";
(globalThis as typeof globalThis & { MonacoEnvironment: unknown }).MonacoEnvironment = {
  getWorker: () => new EditorWorker(),
};
export default function Editor({
  text,
  path,
  theme,
  fontSize,
  fontFamily,
  wordWrap,
  onChange,
  resetKey,
}: {
  text: string;
  path: string;
  theme: Theme;
  fontSize: number;
  fontFamily: string;
  wordWrap: boolean;
  resetKey: string;
  onChange: (text: string) => void;
}) {
  const element = useRef<HTMLDivElement>(null),
    editor = useRef<monaco.editor.IStandaloneCodeEditor | null>(null),
    change = useRef(onChange);
  change.current = onChange;
  useEffect(() => {
    if (!element.current) return;
    const instance = monaco.editor.create(element.current, {
      value: text,
      language: path.endsWith(".json") ? "json" : path.endsWith(".md") ? "markdown" : "plaintext",
      theme: "ag1",
      automaticLayout: true,
      fontFamily: "Geist Mono, monospace",
      fontSize,
      minimap: { enabled: false },
      scrollBeyondLastLine: false,
      padding: { top: 16 },
      ariaLabel: "File editor",
    });
    editor.current = instance;
    const sub = instance.onDidChangeModelContent(() => change.current(instance.getValue()));
    return () => {
      sub.dispose();
      instance.getModel()?.dispose();
      instance.dispose();
      editor.current = null;
    };
  }, [path]);
  useEffect(() => {
    const update = () => {
      const style = getComputedStyle(document.documentElement), color = (key: string) => style.getPropertyValue(`--${key}`).trim();
      const light = !document.documentElement.classList.contains("dark");
      monaco.editor.defineTheme("ag1", { base: light ? "vs" : "vs-dark", inherit: true,
        rules: [{ token: "keyword", foreground: color("link").slice(1) }, { token: "comment", foreground: color("muted-foreground").slice(1) }],
        colors: { "editor.background": color("background"), "editor.foreground": color("foreground"), "editorLineNumber.foreground": color("muted-foreground") } });
      monaco.editor.setTheme("ag1"); editor.current?.updateOptions({ fontSize, fontFamily: fonts[fontFamily as keyof typeof fonts], wordWrap: wordWrap ? "on" : "off" });
    };
    update(); const media = matchMedia("(prefers-color-scheme: dark)"); media.addEventListener?.("change", update);
    return () => media.removeEventListener?.("change", update);
  }, [theme, fontSize, fontFamily, wordWrap, path]);
  useEffect(() => {
    if (editor.current && text !== editor.current.getValue()) editor.current.setValue(text);
  }, [resetKey]);
  return <div className="editor" ref={element} />;
}
