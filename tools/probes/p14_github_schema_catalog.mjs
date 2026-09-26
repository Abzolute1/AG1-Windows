// Deterministic, offline mechanical projection of GitHub's MIT-licensed API
// descriptions into request-only schemas. No credentials or network access.
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import { fileURLToPath } from 'node:url';
const root = fileURLToPath(new URL('../../', import.meta.url));
const bytes = fs.readFileSync(process.argv[2]);
const expected = 'c7fbf17beff9a462c4e099e1de2b5a15ea916d4b6b55e40df640a23285252157';
if (crypto.createHash('sha256').update(bytes).digest('hex') !== expected) throw new Error('Pinned source digest mismatch');
const doc = JSON.parse(bytes);
function resolve(value, chain = []) {
  if (!value || typeof value !== 'object') return value;
  if (chain.length > 24) throw new Error('Schema depth');
  if (value.$ref) {
    if (!value.$ref.startsWith('#/')) throw new Error('External schema reference');
    if (chain.includes(value.$ref)) return { nativeUnsupported: 'recursive schema; bounded server validation only' };
    const target = value.$ref.slice(2).split('/').reduce((v,k) => v[k.replaceAll('~1','/').replaceAll('~0','~')], doc);
    if (!target) throw new Error('Missing local reference');
    return resolve(target, [...chain, value.$ref]);
  }
  if (Array.isArray(value)) return value.map(v => resolve(v, chain));
  const out = {};
  for (const [key, v] of Object.entries(value)) {
    if (['type','nullable','enum','required','minimum','maximum','minLength','maxLength','minItems','maxItems','pattern','format','additionalProperties','allOf','anyOf','oneOf','items'].includes(key)) out[key] = resolve(v,chain);
    else if (key === 'properties') out.properties = Object.fromEntries(Object.entries(v).map(([k,v])=>[k,resolve(v,chain)]));
    else if (key === 'description' && typeof v === 'string') out.description = v.slice(0,240);
  }
  return out;
}
function reference(v) { return v.$ref ? v.$ref.slice(2).split('/').reduce((v,k)=>v[k],doc) : v; }
const operations = [];
for (const [endpoint, item] of Object.entries(doc.paths)) for (const method of ['get','post','put','patch','delete']) {
  const op = item[method]; if (!op) continue;
  const body = op.requestBody ? reference(op.requestBody) : null;
  const jsonMedia = body?.content?.['application/json'];
  const jsonBody = jsonMedia?.schema;
  const parameters = [...(item.parameters ?? []), ...(op.parameters ?? [])].map(reference).filter(p=>p.in==='path'||p.in==='query').map(p=>({ name:p.name, in:p.in, required:!!p.required, schema:resolve(p.schema ?? {}) }));
  operations.push({ id:op.operationId, method:method.toUpperCase(), path:endpoint.slice(1), summary:op.summary ?? op.operationId,
    documentation:op.externalDocs?.url ?? 'https://docs.github.com/en/rest', parameters,
    body:jsonBody ? resolve(jsonBody) : null, bodyRequired:!!body?.required, binaryBody:!!body && !jsonMedia && Object.keys(body.content ?? {}).length > 0 });
}
operations.sort((a,b)=>a.id.localeCompare(b.id,'en'));
const result = { source:'https://raw.githubusercontent.com/github/rest-api-description/main/descriptions/api.github.com/api.github.com.2026-03-10.json',sha256:expected,version:'2026-03-10',license:'MIT, Copyright (c) 2020 GitHub',operations };
const destination=path.join(root,'product/crates/ag1-platform/resources/github-requests-2026-03-10.json');
fs.mkdirSync(path.dirname(destination),{recursive:true});
// This is a generated, mechanically projected data file, not handwritten code.
fs.writeFileSync(destination,JSON.stringify(result)+'\n');
console.log(JSON.stringify({destination,operations:operations.length,bytes:fs.statSync(destination).size,sha256:crypto.createHash('sha256').update(fs.readFileSync(destination)).digest('hex')}));
