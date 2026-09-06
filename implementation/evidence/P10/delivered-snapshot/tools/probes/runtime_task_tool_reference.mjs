// Task metadata and result formatting from the pinned SDK; never executes task.
import fs from 'node:fs';
import path from 'node:path';
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {fileURLToPath,pathToFileURL} from 'node:url';
await import('../offline_guard.mjs');
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../..');
const base=path.join(root,'reference/ag1-executable');
const {createTaskToolDefinition,formatTaskResultContent}=await import(pathToFileURL(path.join(base,'packages/coding-agent/dist/core/tools/task.js')));
const definition=createTaskToolDefinition({cwd:'/fixture',modelRegistry:{},getParentModel:()=>undefined});
const metadata=JSON.parse(JSON.stringify(Object.fromEntries(['name','label','description','parameters','promptSnippet','promptGuidelines'].map(key=>[key,definition[key]]))));
const rows=[];
for(const state of ['completed','failed','timeout','budget']) for(const count of [0,1,11]) {
 const result={state,type:'explore',description:'Inspect the fixture',model:'owned-fixture/initial',summary:count?'Collected findings':'',partial:state!=='completed'&&count>0,evidence:Array.from({length:count},(_,i)=>[{kind:'bash',command:'node --test',exitCode:i?null:0},{kind:'file',action:'write',path:'main.rs'},{kind:'inspect',tool:'read',detail:'main.rs'},{kind:'web',url:'https://example.test/owned'}][i%4]),usage:{input:12000,output:345,cacheRead:0,cacheWrite:0,totalTokens:12345,cost:0},turns:3,durationMs:1500,...(state!=='completed'?{error:'Bounded fixture stopped',checkpointId:'ckpt_fixture'}:{})};
 rows.push({input:result,output:formatTaskResultContent(result)});
}
const files=['packages/coding-agent/src/core/tools/task.ts'];
const captured={version:1,source_commit:'e30f0b4b238ee5618928bcd17b4571eb046e7038',source_files:files.map(file=>({path:file,sha256:createHash('sha256').update(fs.readFileSync(path.join(base,file))).digest('hex')})),metadata,rows,model_actions:0,network_actions:0};
const target=path.join(root,'fixtures/runtime/task-tool-oracle.json');
const resource=path.join(root,'product/crates/ag1-core/resources/runtime-task-tool.json');
if(process.argv.includes('--capture')){
 assert(!fs.existsSync(target));assert(!fs.existsSync(resource));
 fs.writeFileSync(target,JSON.stringify(captured,null,2)+'\n');fs.writeFileSync(resource,JSON.stringify(metadata,null,2)+'\n');
} else {assert.deepEqual(captured,JSON.parse(fs.readFileSync(target,'utf8')));assert.deepEqual(metadata,JSON.parse(fs.readFileSync(resource,'utf8')));}
console.log(JSON.stringify({status:'passed',formats:rows.length,metadata:true,model_actions:0,network_actions:0}));
