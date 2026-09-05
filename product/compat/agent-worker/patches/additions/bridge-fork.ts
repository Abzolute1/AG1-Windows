// P04 B04. Immutable copied roots survive parent deletion; no cross-session search.
import { existsSync, mkdirSync, readFileSync, writeFileSync, cpSync, renameSync, readdirSync, openSync, closeSync, fsyncSync } from 'node:fs';
import { join } from 'node:path';
import { ArtifactStore } from './artifact-store.js';
function syncTree(root: string) {
 for (const entry of readdirSync(root, { withFileTypes: true })) {
  const path = join(root, entry.name); if (entry.isDirectory()) syncTree(path);
  else { const fd = openSync(path, 'r'); try { fsyncSync(fd); } finally { closeSync(fd); } }
 }
 const fd = openSync(root, 'r'); try { fsyncSync(fd); } finally { closeSync(fd); }
}
export function preserveBridgeFork(sessionDir: string, parent: string, child: string, branch: unknown) {
 const source = join(sessionDir, 'artifacts', parent); if (!existsSync(source)) return;
 const target = join(sessionDir, 'artifacts', child); const staging = target + '.staging';
 const serialized = JSON.stringify(branch);
 const ids = new Set(serialized.match(/art_[0-9a-f]{32}/g) ?? []);
 const store = new ArtifactStore(source, parent, { features: { typedEvidenceCursors: true } });
 for (const cursor of serialized.match(/ac[12]\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+/g) ?? []) ids.add(store.decodeReadCursor(cursor).id);
 mkdirSync(join(staging, 'records'), { recursive: true, mode: 0o700 });
 for (const dir of ['blobs', 'indexes']) if (existsSync(join(source, dir))) cpSync(join(source, dir), join(staging, dir), { recursive: true, errorOnExist: true });
 for (const id of ids) { const record = store.get(id); writeFileSync(join(staging, 'records', id + '.json'), JSON.stringify({ ...record, sessionId: child }), { mode: 0o600 }); }
 const origin = join(staging, 'origins', parent); mkdirSync(origin, { recursive: true, mode: 0o700 });
 // Origin records and keys authenticate old cursors, while child records authorize access.
 for (const dir of ['blobs', 'records', 'indexes', 'cursor-key-v1']) if (existsSync(join(source, dir))) cpSync(join(source, dir), join(origin, dir), { recursive: true });
 const ancestors: string[] = existsSync(join(source, 'bridge-origins.json')) ? JSON.parse(readFileSync(join(source, 'bridge-origins.json'), 'utf8')) : [];
 for (const ancestor of ancestors) { if (!/^[a-zA-Z0-9_-]{1,128}$/.test(ancestor)) throw new Error('Invalid origin'); cpSync(join(source, 'origins', ancestor), join(staging, 'origins', ancestor), { recursive: true }); }
 writeFileSync(join(staging, 'bridge-origins.json'), JSON.stringify([...new Set([parent, ...ancestors])]), { mode: 0o600 });
 syncTree(staging); renameSync(staging, target); const fd = openSync(join(sessionDir, 'artifacts'), 'r'); try { fsyncSync(fd); } finally { closeSync(fd); }
}
