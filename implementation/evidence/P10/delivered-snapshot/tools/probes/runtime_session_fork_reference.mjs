// Real pinned SessionManager fork/label behavior in disposable files.
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {fileURLToPath,pathToFileURL} from 'node:url';
await import('../offline_guard.mjs');
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../..');
const source='packages/coding-agent/src/core/session-manager.ts';
const {SessionManager}=await import(pathToFileURL(path.join(root,'reference/ag1-executable/packages/coding-agent/dist/core/session-manager.js')));
const rows=[];
for(const scenario of ['plain','interleaved','updated-off-path','cleared-off-path','outside-target','removed-readded']) {
 const cwd=fs.mkdtempSync(path.join(os.tmpdir(),'ag1-fork-labels-'));
 try {
  const manager=SessionManager.create(cwd,path.join(cwd,'sessions'));
  const names=new Map();
  const put=(name,id)=>{names.set(id,name);return id;};
  const user=(name,text)=>put(name,manager.appendMessage({role:'user',content:text,timestamp:1000}));
  const label=(name,target,value)=>put(name,manager.appendLabelChange(target,value));
  const first=user('first','First instruction');
  if(scenario!=='plain')label('old-label',first,'Original');
  const answer=user('answer','Preserve this continuation');
  const tail=user('tail','Excluded instruction');
  if(scenario==='updated-off-path')label('update',first,'Latest');
  if(scenario==='cleared-off-path')label('clear',first,undefined);
  if(scenario==='outside-target')label('outside',tail,'Do not inherit');
  if(scenario==='removed-readded') {
   label('second-label',answer,'Second');
   label('clear',first,'');
   label('readd',first,'Readded');
  }
  const normalize=entry=>{
   const value=JSON.parse(JSON.stringify(entry));
   value.id=names.get(entry.id)??'missing';
   value.parentId=entry.parentId?(names.get(entry.parentId)??'missing'):null;
   if(entry.targetId)value.targetId=names.get(entry.targetId)??'missing';
   value.timestamp='2026-09-06T00:00:00.000Z';
   return value;
  };
  const input=manager.getEntries().map(normalize);
  manager.createBranchedSession(answer);
  let generated=0;
  for(const entry of manager.getEntries())if(!names.has(entry.id))names.set(entry.id,`fork-label-${++generated}`);
  const entries=manager.getEntries().map(normalize);
  const branch=manager.getBranch().map(normalize);
  const ids=new Set(entries.map(e=>e.id));
  rows.push({scenario,input,leaf:'answer',entries,branch,danglingParents:entries.filter(e=>e.parentId&&!ids.has(e.parentId)).map(e=>e.id)});
 } finally {fs.rmSync(cwd,{recursive:true,force:true});}
}
assert.equal(rows.length,6);
assert.deepEqual(rows.find(r=>r.scenario==='plain').branch.map(e=>e.id),['first','answer']);
for(const row of rows.filter(r=>r.scenario!=='plain')) {
 assert.deepEqual(row.danglingParents,['answer']);
 assert(!row.branch.some(e=>e.id==='first'));
}
assert.equal(rows.find(r=>r.scenario==='updated-off-path').entries.at(-1).label,'Latest');
assert.equal(rows.find(r=>r.scenario==='cleared-off-path').entries.length,2);
assert.deepEqual(rows.find(r=>r.scenario==='removed-readded').entries.filter(e=>e.type==='label').map(e=>e.label),['Second','Readded']);
const result={version:1,source_commit:'e30f0b4b238ee5618928bcd17b4571eb046e7038',source_files:[{path:source,sha256:createHash('sha256').update(fs.readFileSync(path.join(root,'reference/ag1-executable',source))).digest('hex')}],rows,model_actions:0,network_actions:0};
const target=path.join(root,'fixtures/runtime/session-fork-oracle.json');
if(process.argv.includes('--capture')){assert(!fs.existsSync(target));fs.writeFileSync(target,JSON.stringify(result,null,2)+'\n');}
else assert.deepEqual(result,JSON.parse(fs.readFileSync(target,'utf8')));
console.log(JSON.stringify({status:'passed',cases:rows.length,confirmed_dangling_parent_cases:5,model_actions:0,network_actions:0}));
