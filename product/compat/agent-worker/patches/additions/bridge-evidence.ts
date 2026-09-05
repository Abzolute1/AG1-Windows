// P04 B01. Node-only compatibility patch; full evidence is captured before projection.
import { createHash, randomUUID } from 'node:crypto';
import { mkdirSync, openSync, closeSync, writeFileSync, fsyncSync, renameSync } from 'node:fs';
import { join } from 'node:path';
import type { AgentToolResult } from './types.js';
export function capBridgeToolResult<T>(result: AgentToolResult<T>, tokens?: number): AgentToolResult<T> {
 if (!tokens) return result;
 const limit = Math.max(0, Math.floor(tokens * 4));
 const texts = result.content.filter(c => c.type === 'text');
 if (texts.reduce((sum, c) => sum + Buffer.byteLength(c.text), 0) <= limit) return result;
 const root = process.env.AG1_BRIDGE_EVIDENCE;
 if (!root) throw new Error('Tool output requires an evidence destination; refusing lossy projection');
 mkdirSync(root, { recursive: true, mode: 0o700 });
 const raw = Buffer.from(JSON.stringify(result));
 const id = createHash('sha256').update(raw).digest('hex');
 const temporary = join(root, '.' + randomUUID()); const fd = openSync(temporary, 'wx', 0o600);
 try { writeFileSync(fd, raw); fsyncSync(fd); } finally { closeSync(fd); }
 renameSync(temporary, join(root, id + '.json'));
 const directory = openSync(root, 'r'); try { fsyncSync(directory); } finally { closeSync(directory); }
 const marker = `\n[evidence:${id}]`;
 const prefix = (text: string, bytes: number) => {
  const encoded = Buffer.from(text); let end = Math.min(bytes, encoded.length);
  if (end < encoded.length) while (end > 0 && (encoded[end]! & 0xc0) === 0x80) end--;
  return encoded.subarray(0, end).toString('utf8');
 };
 const suffix = prefix(marker, limit); let remaining = limit - Buffer.byteLength(suffix);
 const content = result.content.map(block => {
  if (block.type !== 'text') return block;
  const text = prefix(block.text, remaining); remaining -= Buffer.byteLength(text); return { ...block, text };
 });
 const last = [...content].reverse().find(c => c.type === 'text'); if (last?.type === 'text') last.text += suffix;
 return { ...result, content, details: { ...result.details, bridgeEvidence: id } };
}
