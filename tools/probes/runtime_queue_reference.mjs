// Pinned Agent steering/follow-up queue scheduling; scripted streams, no providers.
import fs from 'node:fs';
import path from 'node:path';
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {fileURLToPath,pathToFileURL} from 'node:url';
await import('../offline_guard.mjs');
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../..');
const base=path.join(root,'reference/ag1-executable');
const {Agent}=await import(pathToFileURL(path.join(base,'packages/agent/dist/index.js')));
const {EventStream}=await import(pathToFileURL(path.join(base,'packages/ai/dist/index.js')));
const message=text=>({role:'user',content:[{type:'text',text}],timestamp:1000});
const assistant=(first)=>({role:'assistant',content:first?[{type:'toolCall',id:'write',name:'write',arguments:{path:'output.txt',content:'first'}}]:[{type:'text',text:'Done'}],api:'openai-completions',provider:'owned-fixture',model:'fixture',usage:{input:0,output:0,cacheRead:0,cacheWrite:0,totalTokens:0,cost:{input:0,output:0,cacheRead:0,cacheWrite:0,total:0}},stopReason:first?'toolUse':'stop',timestamp:1000});
const users=messages=>messages.filter(m=>m.role==='user').map(m=>typeof m.content==='string'?m.content:m.content.filter(b=>b.type==='text').map(b=>b.text).join(''));
const rows=[];
for(const mode of ['one-at-a-time','all']) for(const timing of ['during','before']) {
 let agent;const requests=[];let enqueued=false;
 const enqueue=()=>{enqueued=true;agent.followUp(message('Instruction follow-one'));agent.steer(message('Instruction steer-one'));agent.followUp(message('Instruction follow-two'));agent.steer(message('Instruction steer-two'));};
 agent=new Agent({steeringMode:mode,followUpMode:mode,streamFn:(_model,context)=>{
  assert(requests.length<6,'bounded fixture');requests.push(users(context.messages));
  if(!enqueued)enqueue();
  const stream=new EventStream(e=>e.type==='done'||e.type==='error',e=>e.message??e.error);
  const first=requests.length===1;
  queueMicrotask(()=>stream.push({type:'done',reason:first?'toolUse':'stop',message:assistant(first)}));
  return stream;
 }});
 if(timing==='before')enqueue();
 await agent.prompt('Preserve the original objective');
 rows.push({mode,timing,requests,users:users(agent.state.messages)});
}
const files=['packages/agent/src/agent.ts','packages/agent/src/agent-loop.ts'];
const captured={version:1,source_commit:'e30f0b4b238ee5618928bcd17b4571eb046e7038',source_files:files.map(file=>({path:file,sha256:createHash('sha256').update(fs.readFileSync(path.join(base,file))).digest('hex')})),rows,model_actions:0,network_actions:0};
const target=path.join(root,'fixtures/runtime/queue-oracle.json');
if(process.argv.includes('--capture')){assert(!fs.existsSync(target));fs.writeFileSync(target,JSON.stringify(captured,null,2)+'\n');}
else assert.deepEqual(captured,JSON.parse(fs.readFileSync(target,'utf8')));
console.log(JSON.stringify({status:'passed',queue_cases:rows.length,model_actions:0,network_actions:0}));
