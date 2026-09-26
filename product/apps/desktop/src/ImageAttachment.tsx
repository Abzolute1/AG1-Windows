import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { X, ZoomIn, ZoomOut } from "lucide-react";
import type { Command, JsonValue } from "./generated/protocol";
export type ImageRun = (command: Command) => Promise<JsonValue>;
export const previewableImage = (mime: string) => /^image\/(png|jpeg|webp|gif)$/i.test(mime);

function Preview({ src, name, close, rendition = false }: { src: string; name: string; close: () => void; rendition?: boolean }) {
  const dialog = useRef<HTMLDialogElement>(null), [zoom, setZoom] = useState(false), [size, setSize] = useState("");
  useEffect(() => { const el = dialog.current!; el.showModal(); return () => el.close(); }, []);
  return createPortal(<dialog ref={dialog} className="image-preview" aria-label={`Preview ${name}`} onCancel={e => { e.preventDefault(); close(); }} onClick={e => { if (e.target === e.currentTarget) close(); }}>
    <header><span>{name}<small>{size} · {rendition ? "Raster preview · original retained separately" : "Original image"}</small></span><button type="button" aria-label={zoom ? "Fit image" : "View at full resolution"} onClick={() => setZoom(!zoom)}>{zoom ? <ZoomOut size={20} /> : <ZoomIn size={20} />}</button><button autoFocus type="button" aria-label="Close image preview" onClick={close}><X size={20} /></button></header>
    <div className={`image-preview-canvas${zoom ? " original-size" : ""}`}><img src={src} alt={name} onLoad={e => setSize(`${e.currentTarget.naturalWidth} × ${e.currentTarget.naturalHeight}`)} /></div>
  </dialog>, document.body);
}
export function ImageAttachment({ src, name, remove, inline = false, rendition = false }: { src: string; name: string; remove?: () => void; inline?: boolean; rendition?: boolean }) {
  const [open, setOpen] = useState(false);
  return <div className={`image-attachment${inline ? " inline-image" : ""}`}><button className="image-thumbnail" type="button" aria-label={`Preview ${name}`} onClick={() => setOpen(true)}><img src={src} alt={name} loading="lazy" /></button>{remove && <button className="remove-image" type="button" aria-label={`Remove ${name}`} onClick={remove}><X size={13} /></button>}{open && <Preview src={src} name={name} rendition={rendition} close={() => setOpen(false)} />}</div>;
}
export function DraftAttachment({ file, remove }: { file: File; remove: () => void }) {
  const [src, setSrc] = useState("");
  useEffect(() => { if (!previewableImage(file.type)) return; const url = URL.createObjectURL(file); setSrc(url); return () => URL.revokeObjectURL(url); }, [file]);
  return src ? <ImageAttachment src={src} name={file.name} remove={remove} /> : <button type="button" className="file-attachment" title={file.webkitRelativePath || file.name} onClick={remove}>{file.name}<X size={12} /></button>;
}

// Fetch original image data only when its message approaches the viewport.
// Polling views contain references, not repeated multi-megabyte image payloads.
export function JournalImages({ reference, run, inline = false }: { reference: string; run: ImageRun; inline?: boolean }) {
  const container = useRef<HTMLDivElement>(null);
  const [images, setImages] = useState<{ src: string; name: string; rendition?: boolean }[]>([]), [error, setError] = useState(""), [retry, setRetry] = useState(0);
  const [source, setSource] = useState<{ src: string; name: string }>();
  useEffect(() => {
    let closed = false, started = false;
    const urls: string[] = [];
    async function load() {
      if (started) return; started = true; setError(""); setImages([]); setSource(undefined);
      try {
        let offset: string | null = "0", text = ""; const decoder = new TextDecoder();
        do {
          const page = await run({ kind: "session", action: "entry", args: { id: reference, offset } }) as { base64: string; next: string | null };
          if (closed) return;
          text += decoder.decode(Uint8Array.from(atob(page.base64), c => c.charCodeAt(0)), { stream: page.next !== null });
          if (text.length > 12 * 1024 * 1024) throw new Error("Image message exceeds the preview limit.");
          if (page.next === offset) throw new Error("Image preview could not advance. Try again.");
          offset = page.next;
        } while (offset !== null);
        const message = JSON.parse(text).message, blocks = message.content, preview = message.details?.imagePreview;
        const sourceName = typeof preview?.name === "string" && preview.name.length <= 255 ? preview.name.split(/[\\/]/).at(-1) : undefined;
        const result = (Array.isArray(blocks) ? blocks : []).filter((b: any) => b.type === "image" && previewableImage(b.mimeType)).map((b: any, i: number) => {
          const bytes = Uint8Array.from(atob(b.data), c => c.charCodeAt(0));
          const src = URL.createObjectURL(new Blob([bytes], { type: b.mimeType })); urls.push(src);
          const mcp = message.details?.mcp?.source;
          return { src, rendition: !!mcp, name: mcp ? `MCP ${String(mcp.originalName ?? "image").slice(0, 128)} preview ${i + 1}` : sourceName ? `${sourceName} preview${i ? ` ${i + 1}` : ""}` : `Image ${i + 1}` };
        });
        if (!result.length) throw new Error("No supported image found in this message.");
        if (!closed) {
          setImages(result);
          if (sourceName && preview?.sourceMimeType === "image/svg+xml" && typeof preview.sourceBase64 === "string" && preview.sourceBase64.length <= 350000) {
            const bytes = Uint8Array.from(atob(preview.sourceBase64), c => c.charCodeAt(0));
            // SVG source is download-only data. Never inject it into the page or an iframe.
            const src = URL.createObjectURL(new Blob([bytes], { type: "application/octet-stream" })); urls.push(src);
            setSource({ src, name: sourceName });
          }
        }
      } catch (e) { if (!closed) setError(String(e)); }
    }
    const observer = typeof IntersectionObserver === "undefined" ? null : new IntersectionObserver(entries => { if (entries.some(e => e.isIntersecting)) { observer?.disconnect(); void load(); } }, { rootMargin: "300px" });
    if (observer && container.current) observer.observe(container.current); else void load();
    return () => { closed = true; observer?.disconnect(); urls.forEach(url => URL.revokeObjectURL(url)); };
  }, [reference, run, retry]);
  return <div ref={container} className={`message-images${inline ? " result-images" : ""}`}>{images.map(image => <ImageAttachment key={image.src} {...image} inline={inline} />)}{source && <a className="image-source-download" href={source.src} download={source.name}>Download {source.name}</a>}{!images.length && !error && <span className="attachment-chip">Loading image…</span>}{error && <span role="alert">{error} <button type="button" onClick={() => setRetry(n => n + 1)}>Retry image</button></span>}</div>;
}
