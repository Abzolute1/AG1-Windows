import { memo, useEffect, useMemo, useState, type ReactNode } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { common, createLowlight } from "lowlight";
import { Check, Copy } from "lucide-react";
import "./markdown.css";

const highlighter = createLowlight(common);
type HighlightNode = ReturnType<typeof highlighter.highlight>["children"][number];
function tokens(nodes: HighlightNode[]): ReactNode {
  // Only text and generated spans are rendered. Source code never becomes HTML.
  return nodes.map((node, i) => node.type === "element"
    ? <span key={i} className={Array.isArray(node.properties.className) ? node.properties.className.join(" ") : undefined}>{tokens(node.children)}</span>
    : node.type === "text" ? node.value : null);
}
const CodeBlock = memo(function CodeBlock({ code, language }: { code: string; language: string }) {
  const [notice, setNotice] = useState<{ text: string; ok: boolean } | null>(null);
  useEffect(() => { if (!notice) return; const timer = setTimeout(() => setNotice(null), 2500); return () => clearTimeout(timer); }, [notice]);
  const highlighted = useMemo(() => {
    // Bound highlighting work during streaming; large/unknown blocks remain copyable.
    if (code.length > 32768 || /^(text|txt|plain|plaintext|nohighlight|no-highlight)$/i.test(language)) return null;
    try {
      if (language) return highlighter.registered(language) ? highlighter.highlight(language, code) : null;
      return code.length <= 4096 ? highlighter.highlightAuto(code, { subset: ["javascript", "json", "bash", "python", "rust", "yaml", "sql"] }) : null;
    } catch { return null; }
  }, [code, language]);
  const copied = notice?.text === code && notice.ok;
  async function copy() {
    try { await navigator.clipboard.writeText(code); setNotice({ text: code, ok: true }); }
    catch { setNotice({ text: code, ok: false }); }
  }
  return <div className="code-block">
    <div className="code-block-toolbar"><span>{language || "Code"}</span><button type="button" aria-label={copied ? "Code copied" : "Copy code"} onClick={() => void copy()}>{copied ? <Check size={14} /> : <Copy size={14} />}<span aria-live="polite">{copied ? "Copied" : "Copy"}</span></button></div>
    <pre tabIndex={0} aria-label={language ? `${language} code` : "Code block"}><code>{highlighted ? tokens(highlighted.children) : code}</code></pre>
    {notice?.text === code && !notice.ok && <div className="code-copy-error" role="status">Could not copy. Select the code and copy it manually.</div>}
  </div>;
});
export const Markdown = memo(function Markdown({ text }: { text: string }) {
  return <div className="markdown"><ReactMarkdown remarkPlugins={[remarkGfm]} components={{
    pre: ({ node, children }) => {
      const code = node?.children.find(child => child.type === "element" && child.tagName === "code");
      if (!code || code.type !== "element") return <pre>{children}</pre>;
      const classes = Array.isArray(code.properties.className) ? code.properties.className : [];
      const language = String(classes.find(c => String(c).startsWith("language-")) || "").replace(/^language-/, "");
      return <CodeBlock code={code.children.map(child => child.type === "text" ? child.value : "").join("")} language={language} />;
    },
    a: ({ children, href }) => <a href={href} target="_blank" rel="noopener noreferrer">{children}</a>,
    img: ({ alt }) => <span className="muted">[Image: {alt || "attachment"}]</span>,
  }}>{text}</ReactMarkdown></div>;
});
