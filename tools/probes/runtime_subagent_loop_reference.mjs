// Real pinned delegated runner and owned temporary files; scripted streams only.
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {fileURLToPath,pathToFileURL} from 'node:url';
await import('../offline_guard.mjs');
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../..');
const base=path.join(root,'reference/ag1-executable');
const {runSubagent,resolveSubagentModel}=await import(pathToFileURL(path.join(base,'packages/coding-agent/dist/core/subagent-runner.js')));
const {EventStream}=await import(pathToFileURL(path.join(base,'packages/ai/dist/index.js')));
const model={provider:'owned-fixture',id:'initial',api:'openai-completions',baseUrl:'http://127.0.0.1:1',contextWindow:32768,maxTokens:4096,input:['text'],reasoning:false,cost:{input:2,output:4,cacheRead:1,cacheWrite:2}};
const registry={getAvailable:()=>[model],hasConfiguredAuth:()=>true,find:(provider,id)=>provider===model.provider&&id===model.id?model:undefined};
const rows=[];
for(const type of ['explore','research','implement','verify','plan']) {
 const cwd=fs.mkdtempSync(path.join(os.tmpdir(),'ag1-subagent-loop-'));const requests=[];
 try {
  fs.writeFileSync(path.join(cwd,'existing.txt'),'owned evidence');
  const result=await runSubagent({cwd,modelRegistry:registry,getParentModel:()=>model,getSafetyPolicy:()=>({mode:'disabled',cwd}),streamFnFactory:()=> (_model,context)=>{
   assert(requests.length<2,'bounded source fixture');requests.push({tools:context.tools.map(t=>t.name),roles:context.messages.map(m=>m.role)});
   const first=requests.length===1;
   const message={role:'assistant',content:first?[
    {type:'toolCall',id:'mutate',name:'write',arguments:{path:'child.txt',content:'child result'}},
    {type:'toolCall',id:'inspect',name:'read',arguments:{path:'existing.txt'}},
    {type:'toolCall',id:'recurse',name:'task',arguments:{description:'nested',type:'explore',prompt:'do not recurse'}},
   ]:[{type:'text',text:'Done'}],api:model.api,provider:model.provider,model:model.id,usage:{input:0,output:0,cacheRead:0,cacheWrite:0,totalTokens:0,cost:{input:0,output:0,cacheRead:0,cacheWrite:0,total:0}},stopReason:first?'toolUse':'stop',timestamp:1000};
   const stream=new EventStream(e=>e.type==='done'||e.type==='error',e=>e.message??e.error);
   queueMicrotask(()=>stream.push({type:'done',reason:message.stopReason,message}));return stream;
  }},{type,description:'Inspect the owned fixture',prompt:'Perform the delegated fixture task',timeout:5});
  rows.push({type,requests,state:result.state,summary:result.summary,turns:result.turns,wrote:fs.existsSync(path.join(cwd,'child.txt')),evidence:result.evidence,resultSetEvidence:result.resultSet.allEvidence});
  assert.equal(result.state,'completed');assert.equal(requests.length,2);assert.equal(result.turns,2);
 } finally {fs.rmSync(cwd,{recursive:true,force:true});}
}
const cheap={...model,id:'cheap',cost:{input:0.1,output:0.1,cacheRead:0.1,cacheWrite:0.1}};
const small={...cheap,id:'small',contextWindow:8192};
const routing=[];
for(const type of ['explore','research','implement','verify','plan']) for(const override of [undefined,'owned-fixture/cheap','absent/model','malformed']) {
 const candidates=[model,cheap,small];const catalog={getAvailable:()=>candidates,hasConfiguredAuth:()=>true,find:(provider,id)=>candidates.find(m=>m.provider===provider&&m.id===id)};
 const output=resolveSubagentModel({modelRegistry:catalog,getParentModel:()=>model},{type,model:override});
 routing.push({type,explicit:override??null,parent:model,candidates,selected:output.model??null,error:output.error??null});
}
const files=['packages/coding-agent/src/core/subagent-runner.ts','packages/coding-agent/src/core/model-routing.ts','packages/agent/src/agent-loop.ts'];
const captured={version:1,source_commit:'e30f0b4b238ee5618928bcd17b4571eb046e7038',source_files:files.map(file=>({path:file,sha256:createHash('sha256').update(fs.readFileSync(path.join(base,file))).digest('hex')})),rows,routing,model_actions:0,network_actions:0};
const target=path.join(root,'fixtures/runtime/subagent-loop-oracle.json');
if(process.argv.includes('--capture')){assert(!fs.existsSync(target));fs.writeFileSync(target,JSON.stringify(captured,null,2)+'\n');}
else assert.deepEqual(captured,JSON.parse(fs.readFileSync(target,'utf8')));
console.log(JSON.stringify({status:'passed',profiles:rows.length,routing:routing.length,model_actions:0,network_actions:0}));
