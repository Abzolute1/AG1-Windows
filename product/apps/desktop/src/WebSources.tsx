import { useEffect, useRef, useState } from "react";
import { Globe } from "lucide-react";
import type { ClientMessage } from "./generated/protocol";
import type { ImageRun } from "./ImageAttachment";
export type WebSource = { entry: string; url: string; title: string; host: string };
export function webSources(message: ClientMessage): WebSource[] {
  if (message.tool?.failed || !["webSearch", "webFetch"].includes(message.tool?.name ?? "")) return [];
  const block = message.blocks.find(b => b.kind === "unknown" && (b.value as any)?.kind === "web_result");
  const rows = block?.kind === "unknown" ? (block.value as any)?.sources : null;
  if (!Array.isArray(rows)) return [];
  return rows.slice(0, 4).flatMap(row => {
    try {
      const url = new URL(row.url);
      if (!["https:", "http:"].includes(url.protocol) || url.username || url.password) return [];
      return [{ entry: message.id, url: url.href, title: String(row.title || url.hostname).slice(0, 240), host: url.hostname.replace(/^www\./, "") }];
    } catch { return []; }
  });
}
type CachedIcon = { promise: Promise<string | null>; expires: number };
type IconCache = { values: Map<string, CachedIcon>; active: number; queue: Array<() => void> };
const caches = new WeakMap<ImageRun, IconCache>();
const RETRY_MS = 30_000;
function cacheFor(run: ImageRun) {
  let cache = caches.get(run);
  if (!cache) { cache = { values: new Map(), active: 0, queue: [] }; caches.set(run, cache); }
  return cache;
}
function pump(cache: IconCache) {
  while (cache.active < 4 && cache.queue.length) { cache.active++; cache.queue.shift()!(); }
}
function load(source: WebSource, run: ImageRun) {
  const cache = cacheFor(run), key = new URL(source.url).origin;
  const existing = cache.values.get(key);
  if (existing && existing.expires > Date.now()) return existing.promise;
  let finish!: (value: string | null) => void;
  const record: CachedIcon = { promise: new Promise(resolve => { finish = resolve; }), expires: Infinity };
  cache.values.set(key, record);
  cache.queue.push(() => {
    void Promise.resolve().then(() => run({ kind: "runtime", action: "website_icon", args: { entry: source.entry, url: source.url } })).then(result => {
      const data = (result as any)?.dataUrl;
      return typeof data === "string" && data.length <= 33000 && /^data:image\/png;base64,[A-Za-z0-9+/=]+$/.test(data) ? data : null;
    }).catch(() => null).then(data => {
      record.expires = Date.now() + (data ? 3_600_000 : RETRY_MS);
      finish(data); cache.active--;
      // Never evict pending requests: other cards should share them.
      for (const [oldKey, old] of cache.values) {
        if (cache.values.size <= 128) break;
        if (old.expires !== Infinity) cache.values.delete(oldKey);
      }
      pump(cache);
    });
  });
  pump(cache);
  return record.promise;
}
function SiteIcon({ source, run }: { source: WebSource; run: ImageRun }) {
  const [image, setImage] = useState<string | null>(null), [failed, setFailed] = useState(false);
  const element = useRef<HTMLSpanElement>(null);
  useEffect(() => {
    let current = true, visible = false, loading = false, attempts = 0;
    let timer: ReturnType<typeof setTimeout> | undefined;
    setImage(null); setFailed(false);
    const show = () => {
      if (!current || !visible || loading) return;
      loading = true; attempts++;
      void load(source, run).then(data => {
        loading = false;
        if (!current) return;
        setImage(data);
        if (!data && attempts < 3) timer = setTimeout(show, RETRY_MS + 100);
      });
    };
    const online = () => {
      const cache = cacheFor(run), key = new URL(source.url).origin, record = cache.values.get(key);
      if (record && record.expires !== Infinity && record.expires - Date.now() <= RETRY_MS) cache.values.delete(key);
      clearTimeout(timer); attempts = 0; show();
    };
    const observer = typeof IntersectionObserver === "undefined" ? null : new IntersectionObserver(entries => {
      visible = entries.some(e => e.isIntersecting);
      if (visible) show(); else clearTimeout(timer);
    });
    if (observer && element.current) observer.observe(element.current); else { visible = true; show(); }
    window.addEventListener("online", online);
    return () => { current = false; clearTimeout(timer); observer?.disconnect(); window.removeEventListener("online", online); };
  }, [source.entry, source.url, run]);
  return <span className="site-icon" ref={element} title={source.host} aria-hidden="true">{image && !failed ? <img src={image} width={16} height={16} alt="" onError={() => setFailed(true)} /> : <Globe size={14} />}</span>;
}
export function SiteIcons({ sources, run }: { sources: WebSource[]; run: ImageRun }) {
  const unique = sources.filter((s, i) => sources.findIndex(other => other.host === s.host) === i).slice(0, 3);
  return <span className="site-icons" aria-label={unique.map(s => s.host).join(", ")}>{unique.map(source => <SiteIcon key={source.host} source={source} run={run} />)}</span>;
}
export function SourceList({ sources, run }: { sources: WebSource[]; run: ImageRun }) {
  return <div className="web-sources" aria-label="Web sources">{sources.map(source => <a key={source.url} className="web-source" href={source.url} target="_blank" rel="noopener noreferrer"><SiteIcon source={source} run={run} /><span><strong>{source.title}</strong><small>{source.host}</small></span></a>)}</div>;
}
