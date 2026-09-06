// P10 fake workload: real pinned AgentSession, file tools and owned HTTP codec.
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import http from 'node:http';
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {fileURLToPath,pathToFileURL} from 'node:url';
await import('../offline_guard.mjs');
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../..');
const base=path.join(root,'reference/ag1-executable');
const load=relative=>import(pathToFileURL(path.join(base,relative)));
const {Agent}=await load('packages/agent/dist/index.js');
const {streamSimple,getModel}=await load('packages/ai/dist/index.js');
const {AgentSession}=await load('packages/coding-agent/dist/core/agent-session.js');
const {SessionManager}=await load('packages/coding-agent/dist/core/session-manager.js');
const {SettingsManager}=await load('packages/coding-agent/dist/core/settings-manager.js');
const {ModelRegistry}=await load('packages/coding-agent/dist/core/model-registry.js');
const {AuthStorage}=await load('packages/coding-agent/dist/core/auth-storage.js');
const {convertToLlm}=await load('packages/coding-agent/dist/core/messages.js');
const {createExtensionRuntime}=await load('packages/coding-agent/dist/core/extensions/loader.js');
const content='Owned evidence β\n'.repeat(256),samples=[],cycles=[];
let requests=[],readFirst=false;
const server=http.createServer(async(req,res)=>{
 let body='';for await(const part of req)body+=part;
 const request=JSON.parse(body);requests.push(request);
 const tool=readFirst&&requests.length===1;
 const reply={id:'owned',object:'chat.completion.chunk',created:1000,model:'initial',choices:[{index:0,delta:tool?{tool_calls:[{index:0,id:'read-1',type:'function',function:{name:'read',arguments:'{"path":"input.txt"}'}}]}:{content:'Done'},finish_reason:tool?'tool_calls':'stop'}],usage:{prompt_tokens:100,completion_tokens:2}};
 const output=`data: ${JSON.stringify(reply)}\n\ndata: [DONE]\n\n`;
 res.writeHead(200,{'Content-Type':'text/event-stream','Content-Length':Buffer.byteLength(output),'Connection':'close'});res.end(output);
});
await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
const endpoint=`http://127.0.0.1:${server.address().port}`;
const model={...getModel('openai','gpt-4o-mini'),api:'openai-completions',provider:'owned-fixture',id:'initial',baseUrl:endpoint,contextWindow:32768,maxTokens:4096,reasoning:false,input:['text']};
function session(cwd,manager,times,onFirst){
 const auth=AuthStorage.inMemory();auth.setRuntimeApiKey('owned-fixture','owned-fixture');
 const resources={getExtensions:()=>({extensions:[],errors:[],runtime:createExtensionRuntime()}),getSkills:()=>({skills:[],diagnostics:[]}),getPrompts:()=>({prompts:[],diagnostics:[]}),getThemes:()=>({themes:[],diagnostics:[]}),getAgentsFiles:()=>({agentsFiles:[]}),getSystemPrompt:()=> 'Owned native fixture',getAppendSystemPrompt:()=>[],extendResources:()=>{},reload:async()=>{}};
 const agent=new Agent({convertToLlm,getApiKey:()=> 'owned-fixture',initialState:{model,systemPrompt:'Owned native fixture',messages:manager.buildSessionContext().messages},streamFn:(...args)=>{times?.push(process.hrtime.bigint());if(times?.length===1)onFirst?.();return streamSimple(...args);}});
 return new AgentSession({agent,agentDir:path.join(cwd,'agent'),cwd,sessionManager:manager,settingsManager:SettingsManager.inMemory({visualTools:"off",compaction:{enabled:false,reserveTokens:8192,keepRecentTokens:1},retry:{enabled:false}}),modelRegistry:ModelRegistry.create(auth,cwd),resourceLoader:resources,initialActiveToolNames:['read','write','append']});
}
try {
 for(let i=0;i<25;i++){
  const cwd=fs.mkdtempSync(path.join(os.tmpdir(),'ag1-p10-ts-'));let s;
  try{
   fs.writeFileSync(path.join(cwd,'input.txt'),content);
   const manager=SessionManager.create(cwd,path.join(cwd,'sessions')),times=[];s=session(cwd,manager,times,()=>{void s.followUp("Continue");});requests=[];readFirst=true;
   const start=process.hrtime.bigint();await s.prompt('Read input.txt');await s.waitForEventLoop();
   const total=Number(process.hrtime.bigint()-start);
   assert.equal(requests.length,3,JSON.stringify({requests:requests.map(r=>r.messages),branch:manager.getBranch()}));assert.equal(times.length,3);
   const results=requests[1].messages.filter(m=>m.role==='tool');assert.equal(results.length,1);assert(results[0].content.includes(content.trimEnd()));assert(JSON.stringify(requests[2].messages).includes('Continue'));
   const branch=manager.getBranch();const resultsInBranch=branch.filter(e=>e.type==='message'&&e.message.role==='toolResult');assert.equal(resultsInBranch.length,1);assert.notEqual(resultsInBranch[0].message.isError,true);
   samples.push({total_ns:total,first_request_ns:Number(times[0]-start),tool_context_ns:Number(times[1]-times[0]),followup_ns:Number(times[2]-times[1]),requests:3,tool_results:1,full_read_evidence:true,users:['Read input.txt','Continue']});
  }finally{s?.dispose();fs.rmSync(cwd,{recursive:true,force:true});}
 }
 const cwd=fs.mkdtempSync(path.join(os.tmpdir(),'ag1-p10-ts-cycles-'));let s;
 try{
  let manager=SessionManager.create(cwd,path.join(cwd,'sessions'));
  manager.appendMessage({role:'user',content:'Original task and evidence '.repeat(200),timestamp:0});
  manager.appendMessage({role:'assistant',content:[{type:'text',text:'Earlier findings are recorded.'}],stopReason:'stop',api:model.api,provider:model.provider,model:model.id,usage:{input:100,output:2,cacheRead:0,cacheWrite:0,totalTokens:102,cost:{input:0,output:0,cacheRead:0,cacheWrite:0,total:0}},timestamp:0});
  s=session(cwd,manager);readFirst=false;requests=[];
  for(let i=0;i<32;i++){
   const start=process.hrtime.bigint();await s.prompt(`Continue cycle ${i}`);await s.waitForEventLoop();await s.compact();await s.waitForEventLoop();
   const file=manager.getSessionFile();s.dispose();manager=SessionManager.open(file,path.join(cwd,'sessions'));s=session(cwd,manager);
   globalThis.gc();
   cycles.push({cycle:i,ns:Number(process.hrtime.bigint()-start),heap_bytes:process.memoryUsage().heapUsed,journal_entries:manager.getEntries().length,projected_messages:manager.buildSessionContext().messages.length});
  }
  assert.equal(requests.length,96);
 }finally{s?.dispose();fs.rmSync(cwd,{recursive:true,force:true});}
 fs.writeFileSync(process.argv[2],JSON.stringify({status:'measured',runtime:'typescript',samples,cycles,input_sha256:createHash('sha256').update(content).digest('hex'),model_actions:0,external_network_actions:0,scope:'Pinned AgentSession with native reference file tools and owned loopback OpenAI SSE. JSONL durability differs from Rust FULL SQL plus original-input/output custody; timings are host workload observations, not equivalent durability or a model speedup. 32 cumulative resume/compaction cycles reopen the session file; V8 heapUsed sampled after explicit GC.'},null,2)+'\n');
}finally{await new Promise(resolve=>server.close(resolve));}
console.log('Measured pinned session tool/context/follow-up and 32 resume/compaction cycles');
