import { useEffect, useRef, useState } from "react";
import { ArrowLeft, Search, RefreshCw } from "lucide-react";
import { cachedIcon, discoverCache, loadIcon, type DiscoverCache } from "./MCPDiscoverCache";
const literal = (value: unknown, fallback = "") => typeof value === "string" ? value : fallback;
const array = (value: unknown): any[] => Array.isArray(value) ? value : [];
const identifier = (entry: any) => literal(entry.server.name);
const name = (entry: any) => literal(entry.server.title) || identifier(entry).split("/").at(-1)!.split(/[-_]/).map(w => w.toLowerCase() === "mcp" ? "MCP" : w.charAt(0).toUpperCase() + w.slice(1)).join(" ");
const publisher = (entry: any) => (identifier(entry).split("/")[0] || "Not supplied").replace(/^io\.github\./, "");
const official = (entry: any) => entry.registryMeta?.["io.modelcontextprotocol.registry/official"];
const published = (entry: any) => { const date = Date.parse(literal(official(entry)?.publishedAt)); return Number.isFinite(date) ? date : 0; };
const remote = (entry: any) => entry.summary?.remote ?? array(entry.server.remotes).length > 0;
const localPackage = (entry: any) => entry.summary?.local ?? array(entry.server.packages).length > 0;
const transport = (entry: any) => entry.summary?.transport ?? ([...new Set([...array(entry.server.remotes).map(r => literal(r?.type)), ...array(entry.server.packages).map(p => literal(p?.transport?.type))].filter(Boolean))].join(" · ") || "Not supplied");
const privateValues = (entry: any) => entry.summary?.credentials ?? [...array(entry.server.remotes).flatMap(r => array(r?.headers)), ...array(entry.server.packages).flatMap(p => array(p?.environmentVariables))].some(v => v?.isSecret === true);
function ServerIcon({ entry, image }: { entry: any; image?: string }) { const [failed, setFailed] = useState(false); useEffect(() => setFailed(false), [image]); return <span className="mcp-server-icon" aria-hidden="true">{image && !failed ? <img src={image} alt="" width={52} height={52} onError={() => setFailed(true)} /> : name(entry).slice(0, 2).toUpperCase()}</span>; }
const connection = (entry: any) => [localPackage(entry) && "Local", remote(entry) && "Remote"].filter(Boolean).join(" · ") || "Not specified";

type Props = { cache?: DiscoverCache; invoke: (args: any) => Promise<any>; action: (work: () => Promise<void>) => Promise<void>; busy: boolean; added: (id: string, message: string) => void; manual: () => void };
const when = (seconds: number) => new Date(seconds * 1000).toLocaleString();
export function MCPDiscover({ invoke, action, busy, added, manual, cache = discoverCache }: Props) {
  const initial = useRef(cache.directory).current;
  const [query, setQuery] = useState(""), [view, setView] = useState<any>(initial?.view ?? null), [detail, setDetail] = useState<any>(null), [review, setReview] = useState<any>(null), [ack, setAck] = useState(false);
  const [local, setLocal] = useState<any>(null), [executable, setExecutable] = useState(""), [argumentsText, setArguments] = useState("[]"), [cwd, setCwd] = useState(""), [environment, setEnvironment] = useState("{}");
  const [entries, setEntries] = useState<any[]>(initial?.entries ?? []), [category, setCategory] = useState("all");
  const [loading, setLoading] = useState(!initial);
  const [, redrawIcons] = useState(0);
  useEffect(() => {
    let active = true, next = 0;
    const missing = entries.filter(entry => entry.iconUrl && !cachedIcon(cache, entry.iconUrl));
    async function worker() {
      while (active && next < missing.length) {
        const entry = missing[next++];
        await loadIcon(cache, entry, invoke);
        if (active) redrawIcons(value => value + 1);
      }
    }
    void Promise.all(Array.from({ length: Math.min(4, missing.length) }, worker));
    return () => { active = false; };
  }, [entries, cache]);
  function display(data: any, append = false) {
    setView((old: any) => data.page ? data : { ...data, page: old?.page });
    if (data.page) setEntries(old => {
      const entries = append ? [...new Map([...old, ...data.page.entries].map(e => [e.key, e])).values()].slice(0, 320) : data.page.entries;
      if (!data.query) cache.directory = { view: data, entries, firstFetchedAt: append ? cache.directory?.firstFetchedAt ?? data.page.fetchedAt : data.page.fetchedAt, savedAt: Date.now() };
      return entries;
    });
  }
  const heading = useRef<HTMLHeadingElement>(null);
  const pending = useRef<string | null>(null), generation = useRef(0);
  useEffect(() => { if (detail) heading.current?.focus(); }, [detail?.key]);
  useEffect(() => {
    const ticket = ++generation.current;
    queueMicrotask(() => {
      if (ticket !== generation.current) return;
      void action(async () => {
      try {
        const cached = await invoke({ action: "discover", fetch: false });
        if (ticket !== generation.current) return;
        const retained = initial && cached.page && initial.firstFetchedAt === cached.page.fetchedAt && Date.now() - initial.savedAt < 300000;
        if (!retained) display(cached);
        // Opening Discover requests public directory metadata only. No query,
        // project context, credential, advertised endpoint or process is involved.
        if (!cached.page || cached.stale || cached.page.iconVersion !== 1) {
          const data = await invoke({ action: "discover", query: "", fetch: true, ...(cached.page && cached.page.iconVersion !== 1 ? { refresh: true } : {}) });
          if (ticket === generation.current) display(data);
        }
      } finally { if (ticket === generation.current) setLoading(false); }
      });
    });
    return () => { ++generation.current; const id = pending.current; pending.current = null; if (id) void invoke({ action: "discover_dismiss", review: id }).catch(() => {}); };
  }, []);
  async function search(cursor = "", refresh = false, searchQuery = "") {
    const ticket = generation.current;
    setLoading(true);
    try {
      const data = await invoke({ action: "discover", query: searchQuery, cursor, fetch: true, refresh });
      if (ticket !== generation.current) return;
      display(data, !!cursor);
      if (!data.error) { setDetail(null); setLocal(null); }
    } finally { if (ticket === generation.current) setLoading(false); }
  }
  async function inspect(key: string) {
    const ticket = generation.current;
    const data = await invoke({ action: "discover_detail", key });
    if (ticket === generation.current) { setDetail(data); setLocal(null); }
  }
  async function prepare(index: number) { const r = await invoke({ action: "discover_review", key: detail.key, sha256: detail.sha256, index }); pending.current = r.review; setReview(r); setAck(false); }
  async function prepareLocal() {
    const args = JSON.parse(argumentsText), publicEnvironment = JSON.parse(environment);
    if (!Array.isArray(args) || args.some(v => typeof v !== "string")) throw new Error("Arguments must be a JSON array of strings.");
    if (!publicEnvironment || Array.isArray(publicEnvironment) || typeof publicEnvironment !== "object" || Object.values(publicEnvironment).some(v => typeof v !== "string")) throw new Error("Public environment must be a JSON object of strings.");
    const r = await invoke({ action: "discover_local_review", key: detail.key, sha256: detail.sha256, index: local.index, configuration: { transport: "local", executable, arguments: args, cwd, public_environment: publicEnvironment, secret_environment: [] } });
    pending.current = r.review; setReview(r); setAck(false);
  }
  async function dismiss() { const id = pending.current; pending.current = null; setReview(null); if (id) await invoke({ action: "discover_dismiss", review: id }); }
  async function save() { const id = pending.current; pending.current = null; setReview(null); if (id) { const result = await invoke({ action: "discover_confirm", review: id }); added(result.ids[0], result.message); } }
  const normalizedQuery = query.trim().toLocaleLowerCase();
  const filtered = entries.filter(entry => (!normalizedQuery || `${name(entry)} ${identifier(entry)} ${publisher(entry)} ${literal(entry.server.description)} ${transport(entry)}`.toLocaleLowerCase().includes(normalizedQuery)) && (category === "all" || (category === "remote" ? remote(entry) : localPackage(entry))));
  const recent = !normalizedQuery && category === "all" ? [...filtered].filter(e => published(e) > 0).sort((a, b) => published(b) - published(a)).slice(0, 3) : [];
  const remaining = filtered.filter(e => !recent.some(r => r.key === e.key));
  function rows(items: any[]) { return items.map(entry => <article key={entry.key} className={`mcp-directory-row${detail?.key === entry.key ? " is-selected" : ""}`}>
    <button className="mcp-row-main" aria-pressed={detail?.key === entry.key} disabled={busy || !!review} onClick={() => void action(() => inspect(entry.key))}>
      <ServerIcon entry={entry} image={cachedIcon(cache, entry.iconUrl)?.data} /><span className="mcp-row-copy"><span className="mcp-row-title"><strong>{name(entry)}</strong></span><span className="mcp-row-description">{literal(entry.server.description, "No description supplied.")}</span>
      <span className="mcp-row-meta"><span>{publisher(entry)}</span>{literal(entry.server.version) && <span>v{literal(entry.server.version)}</span>}<span>{connection(entry)}</span>{privateValues(entry) && <span>Credentials required</span>}{official(entry)?.status && official(entry).status !== "active" && <span>{literal(official(entry).status)}</span>}</span></span>
    </button><button className="mcp-row-add" aria-label={`Add ${name(entry)}`} title="Inspect setup options before adding" disabled={busy || !!review} onClick={() => void action(() => inspect(entry.key))}>Add</button>
  </article>); }
  return <section aria-label="Discover MCP servers" className="mcp-discover">
    <div className="mcp-discover-toolbar"><label className="settings-search"><Search size={16} aria-hidden="true" /><input aria-label="Search public servers" placeholder="Filter servers by name, publisher or description…" maxLength={128} value={query} onChange={e => setQuery(e.target.value)} /></label><button type="button" className="quiet-button" disabled={busy || !!review} onClick={() => void action(() => search("", true))}><RefreshCw size={14} aria-hidden="true" />Refresh source</button><button type="button" disabled={busy || !!review} onClick={manual}>Add manually</button></div>
    <details className="mcp-discover-about"><summary>About Discover{view?.stale ? " · Cached results are stale" : ""}</summary><p>Browse the GitHub MCP Registry. Opening Discover loads public listings; filtering stays on this device. “Search registry” sends only the text in this search field. Server endpoints are never contacted while browsing.</p><p>Listings are publisher claims, not AG1 verification. Adding a server saves it inactive; connection, authentication and tool permissions are reviewed separately. Publisher icons come from registry-provided GitHub avatar metadata. Native code fetches and rasterizes them without credentials; unavailable icons use monograms. Featured rankings, popularity and tool counts are not inferred.</p><p>{view?.coverage} Categories reflect listed connection types. Recently added uses registry publication dates among loaded listings, not a directory-wide ranking.</p>{view?.page && <p>Retrieved {when(view.page.fetchedAt)} · {view.stale ? "Stale cache; refresh before setup" : "Cached for up to five minutes"}.</p>}{!!view?.page?.excluded && <p>{view.page.excluded} oversized or unsupported records were omitted from this page.</p>}</details>
    {view?.error && <p role="status" className="mcp-discover-status">{view.error}</p>}
    {review ? <section className="mcp-review" aria-label="Review discovered server"><h2>Review server setup</h2><p>{review.message}</p><pre className="mcp-source">{JSON.stringify(review.draft, null, 2)}</pre><label className="checkbox-label"><input type="checkbox" checked={ack} onChange={e => setAck(e.target.checked)} />I reviewed this target and source.</label><div className="mcp-actions"><button disabled={busy} onClick={() => void action(dismiss)}>Cancel setup</button><button disabled={busy || !ack} onClick={() => void action(save)}>Save discovered server inactive</button></div></section> :
      <div className={`mcp-directory-layout${detail ? " has-detail" : ""}`}>
        <div className="mcp-directory" aria-label="Public MCP listings">
          <div className="mcp-directory-controls"><h2>Directory <span>{filtered.length}</span></h2><label>Categories<select aria-label="Server category" value={category} onChange={e => setCategory(e.target.value)}><option value="all">All servers</option><option value="remote">Remote services</option><option value="local">Local packages</option></select></label></div>
          {loading && <p role="status" className="mcp-discover-status">Loading public directory…</p>}
          {recent.length > 0 && <section aria-label="Recently added"><h3 className="mcp-directory-section">Recently added <small>In this directory selection</small></h3>{rows(recent)}</section>}
          {remaining.length > 0 && <section aria-label="Directory results">{recent.length > 0 && <h3 className="mcp-directory-section">More servers</h3>}{rows(remaining)}</section>}
          {!loading && !filtered.length && <div className="mcp-directory-empty"><h3>{entries.length ? "No matching servers" : "The directory is unavailable"}</h3><p>{entries.length ? "Try another filter, load more listings or search the registry." : "Refresh the source to try again, or add a server manually."}</p>{(query || category !== "all") && <button onClick={() => { setQuery(""); setCategory("all"); }}>Clear filters</button>}</div>}
          <footer className="mcp-directory-footer"><span>{entries.length} loaded{view?.page?.nextCursor ? " · More available" : ""}</span><div>{view?.page?.nextCursor && entries.length < 320 && <button disabled={busy} onClick={() => void action(() => search(view.page.nextCursor, false, view.query ?? ""))}>Load more</button>}{normalizedQuery && <button disabled={busy} onClick={() => void action(() => search("", false, query))}>Search registry</button>}</div></footer>
        </div>
        <div className="mcp-details mcp-directory-detail">{detail ? <>
          <div className="mcp-detail-navigation"><button className="quiet-button" onClick={() => { setDetail(null); setLocal(null); }}><ArrowLeft size={14} />Back to directory</button></div>
          <header className="mcp-detail-heading"><ServerIcon entry={detail} image={cachedIcon(cache, detail.iconUrl)?.data} /><div><h2 ref={heading} tabIndex={-1}>{name(detail)}</h2><span>{publisher(detail)}</span></div></header><p className="mcp-detail-description">{literal(detail.server.description)}</p>
          <dl className="mcp-detail-facts"><div><dt>Publisher</dt><dd>{publisher(detail)}</dd></div><div><dt>Registry ID</dt><dd>{identifier(detail)}</dd></div><div><dt>Version</dt><dd>{literal(detail.server.version, "Not supplied")}</dd></div><div><dt>Transport</dt><dd>{transport(detail)}</dd></div><div><dt>Authentication</dt><dd>{privateValues(detail) ? "Private values listed; enter them after adding" : "Not specified; checked on connection"}</dd></div><div><dt>Registry status</dt><dd>{literal(official(detail)?.status, "Unknown")} · Not verified by AG1</dd></div></dl>
          <section className="mcp-detail-permissions"><h3>Capabilities & permissions</h3><p>{literal(detail.capabilities)}</p><p>Tools require a separate permission review. The server is saved inactive.</p></section>
          {detail.setup.remote.map((recipe: any) => <section key={recipe.index}><h3>Remote connection</h3><p className="mcp-endpoint">{literal(recipe.target.url, "Endpoint not supplied")}</p><details><summary>Endpoint configuration</summary><pre className="mcp-source">{JSON.stringify(recipe.target, null, 2)}</pre></details><p>{recipe.guidance}</p>{recipe.supported && <button disabled={busy || detail.stale} onClick={() => void action(() => prepare(recipe.index))}>Review this remote</button>}</section>)}
          {detail.setup.local.map((recipe: any, i: number) => <section key={i}><h3>Local package</h3><p className="mcp-endpoint">{literal(recipe.package.identifier, "Package not supplied")} · {literal(recipe.package.version, "Version not supplied")}</p><details><summary>Package configuration</summary><pre className="mcp-source">{JSON.stringify(recipe.package, null, 2)}</pre></details><p>{recipe.guidance}</p>{recipe.supported ? <button disabled={busy || detail.stale} onClick={() => { setLocal(recipe); setExecutable(""); setArguments("[]"); setEnvironment("{}"); setCwd(detail.suggestedDirectory || ""); }}>Configure existing local installation</button> : <button disabled={busy} onClick={manual}>Add manually</button>}</section>)}
          {!detail.setup.remote.length && !detail.setup.local.length && <p>No guided setup recipe supplied. <button disabled={busy} onClick={manual}>Add manually</button></p>}
          {local && <form className="mcp-form" onSubmit={e => { e.preventDefault(); void action(prepareLocal); }}><h3>Existing local installation</h3><p>Package: {String(local.package.identifier ?? "Unspecified")} · Version {String(local.package.version ?? "Unspecified")}. Install using the publisher’s instructions first. AG1 will not download or install this package. Enter its installed executable and literal arguments.</p><label>Installed executable<input required value={executable} onChange={e => setExecutable(e.target.value)} maxLength={4096} /></label><label>Installed arguments · JSON array<textarea value={argumentsText} onChange={e => setArguments(e.target.value)} maxLength={16000} /></label><label>Reviewed working directory<input required value={cwd} onChange={e => setCwd(e.target.value)} maxLength={4096} /></label><label>Required public environment · JSON object<textarea value={environment} onChange={e => setEnvironment(e.target.value)} maxLength={16000} /></label><p>Private environment names from the listing will be saved for native private entry. Do not enter their values here. The initial access mode is read-only directory, with no network.</p><button disabled={busy}>Review local setup</button></form>}
          <details><summary>Registry metadata and provenance</summary><pre className="mcp-source">{JSON.stringify({ server: detail.server, registryMeta: detail.registryMeta, sha256: detail.sha256 }, null, 2)}</pre></details></> : <div className="mcp-detail-empty"><h2>Select a server</h2><p>Review its description, publisher and connection options here.</p><p>You can inspect capabilities, authentication and permissions before choosing how to add it. Nothing connects until you review the setup.</p></div>}</div></div>}
  </section>;
}
