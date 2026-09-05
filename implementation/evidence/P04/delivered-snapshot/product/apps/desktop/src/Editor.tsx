import { useEffect, useRef } from "react";
import * as monaco from "monaco-editor/esm/vs/editor/editor.api";
import EditorWorker from "monaco-editor/esm/vs/editor/editor.worker?worker";
(globalThis as typeof globalThis & { MonacoEnvironment: unknown }).MonacoEnvironment = {
  getWorker: () => new EditorWorker(),
};
export default function Editor({
  text,
  path,
  dark,
  onChange,
  resetKey,
}: {
  text: string;
  path: string;
  dark: boolean;
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
      theme: dark ? "vs-dark" : "vs",
      automaticLayout: true,
      fontFamily: "Geist Mono, monospace",
      fontSize: 13,
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
    monaco.editor.setTheme(dark ? "vs-dark" : "vs");
  }, [dark]);
  useEffect(() => {
    if (editor.current && text !== editor.current.getValue()) editor.current.setValue(text);
  }, [resetKey]);
  return <div className="editor" ref={element} />;
}
