// Public display data only. Configuration, reviews and credentials never enter
// this cache. Keep it for the renderer lifetime so reopening Discover is instant.
export function createDiscoverCache() {
  return { directory: null as null | { view: any; entries: any[]; firstFetchedAt: number; savedAt: number },
    icons: new Map<string, { data?: string; until: number }>(),
    requests: new Map<string, Promise<string | undefined>>() };
}
export const discoverCache = createDiscoverCache();
export type DiscoverCache = ReturnType<typeof createDiscoverCache>;
export function cachedIcon(cache: DiscoverCache, source: string) {
  const item = cache.icons.get(source);
  return item && item.until > Date.now() ? item : undefined;
}
export function loadIcon(cache: DiscoverCache, entry: any, invoke: (args: any) => Promise<any>) {
  const source = entry.iconUrl;
  const cached = cachedIcon(cache, source);
  if (cached) return Promise.resolve(cached.data);
  const pending = cache.requests.get(source);
  if (pending) return pending;
  const request = invoke({ action: "discover_icon", key: entry.key }).then(result => {
    const data = result?.dataUrl;
    return typeof data === "string" && data.length <= 33000 && /^data:image\/png;base64,[A-Za-z0-9+/]+={0,2}$/.test(data) ? data : undefined;
  }).catch(() => undefined).then(data => {
    cache.icons.delete(source);
    cache.icons.set(source, { data, until: Date.now() + (data ? 86400000 : 300000) });
    while (cache.icons.size > 128) cache.icons.delete(cache.icons.keys().next().value!);
    return data;
  }).finally(() => cache.requests.delete(source));
  cache.requests.set(source, request);
  return request;
}
