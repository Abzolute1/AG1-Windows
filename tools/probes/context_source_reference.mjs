import fs from 'node:fs';import path from 'node:path';import os from 'node:os';import assert from 'node:assert/strict';import {fileURLToPath,pathToFileURL} from 'node:url';
await import('../offline_guard.mjs');
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../..');
const {ArtifactStore}=await import(pathToFileURL(path.join(root,'reference/ag1-executable/packages/coding-agent/dist/core/artifact-store.js')));
const {createSourceContextToolDefinition}=await import(pathToFileURL(path.join(root,'reference/ag1-executable/packages/coding-agent/dist/core/tools/source-context.js')));
const fixture=JSON.parse(fs.readFileSync(path.join(root,'fixtures/context/source-inputs.json'),'utf8')),groups=[];
for(const scenario of fixture.cases){const temporary=fs.mkdtempSync(path.join(os.tmpdir(),'ag1-source-reference-'));try{
 const workspace=path.join(temporary,'workspace');fs.mkdirSync(workspace);
 for(const [name,text] of Object.entries(fixture.files)){const file=path.join(workspace,name);fs.mkdirSync(path.dirname(file),{recursive:true});fs.writeFileSync(file,text);}
 if(scenario.long)fs.writeFileSync(path.join(workspace,'src/long.ts'),'export const longSource = "'+'x'.repeat(5000)+'";\n'+Array.from({length:100},(_,i)=>`// line ${i}`).join('\n'));
 const branch=scenario.recent?[{type:'message',id:'entry1',message:{role:'assistant',content:[{type:'toolCall',id:'old-read',name:'read',arguments:{path:scenario.recent}}]}}]:[];
 const journal=[];const store=new ArtifactStore(path.join(temporary,'artifacts'),'fixture');
 const deps={artifactStore:store,sessionManager:{getBranch:()=>[...branch,...journal],appendCustomEntryAsync:async(kind,data)=>{const id=`record${journal.length}`;journal.push({type:'custom',id,customType:kind,data});return id;}},collectWorkingTreeSignals:async()=>({insideRepository:!!scenario.changed,changedFiles:new Set(scenario.changed??[]),changedLines:new Map((scenario.changed??[]).map(p=>[p,new Set([1])]))})};
 const result=await createSourceContextToolDefinition(workspace,deps).execute('source-call',scenario.input);
 const id=result.details.artifactId;const normalized=JSON.parse(JSON.stringify(result).replaceAll(id,'art_00000000000000000000000000000001'));
 const saved=journal[0].data;delete saved.createdAt;saved.artifactId='art_00000000000000000000000000000001';
 for(const p of scenario.required)assert(result.details.files.some(f=>f.path===p));for(const p of scenario.forbidden)assert(!result.details.files.some(f=>f.path===p));
 groups.push({id:scenario.id,result:normalized,record:saved});
 }finally{fs.rmSync(temporary,{recursive:true,force:true});}}
const out={version:1,source_commit:'e30f0b4b238ee5618928bcd17b4571eb046e7038',model_actions:0,network_actions:0,groups};const target=path.join(root,'fixtures/context/source-oracle.json');if(process.argv.includes('--capture')){assert(!fs.existsSync(target));fs.writeFileSync(target,JSON.stringify(out,null,2)+'\n');}else assert.deepEqual(out,JSON.parse(fs.readFileSync(target,'utf8')));console.log(JSON.stringify({status:'passed',source_cases:groups.length,model_actions:0,network_actions:0}));
