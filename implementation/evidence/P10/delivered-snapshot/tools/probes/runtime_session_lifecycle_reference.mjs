// Pinned runtime replacement and real SessionManager files; session services
// are bounded host stubs, not a claim of full AgentSession lifecycle parity.
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {fileURLToPath,pathToFileURL} from 'node:url';
await import('../offline_guard.mjs');
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../..');
const base=path.join(root,'reference/ag1-executable');
const {AgentSessionRuntime}=await import(pathToFileURL(path.join(base,'packages/coding-agent/dist/core/agent-session-runtime.js')));
const {SessionManager}=await import(pathToFileURL(path.join(base,'packages/coding-agent/dist/core/session-manager.js')));
const priorCwd=process.cwd();
const rows=[];
for(const scenario of ['new','resume','fork-first','fork-later','cancel-new','cancel-fork','prepare-failure','shutdown-failure','invalid-fork']) {
 const cwd=fs.mkdtempSync(path.join(os.tmpdir(),'ag1-native-lifecycle-'));
 try {
  const events=[];const files=new Map();const ids=new Map();
  const dir=path.join(cwd,'sessions');
  const manager=SessionManager.create(cwd,dir);files.set(manager.getSessionFile(),'initial');
  const message=(role,text)=>role==='user'?{role,content:text,timestamp:1000}:{role,content:[{type:'text',text}],api:'openai-completions',provider:'fixture',model:'initial',usage:{input:0,output:0,cacheRead:0,cacheWrite:0,totalTokens:0,cost:{input:0,output:0,cacheRead:0,cacheWrite:0,total:0}},stopReason:'stop',timestamp:1000};
  const first=manager.appendMessage(message('user','First instruction'));
  const answer=manager.appendMessage(message('assistant','First answer'));
  const later=manager.appendMessage(message('user',[{type:'text',text:'Second'},{type:'image',data:'ignored',mimeType:'image/png'},{type:'text',text:'instruction'}]));
  manager.appendMessage(message('assistant','Second answer'));
  ids.set(first,'first-user');ids.set(later,'later-user');ids.set(answer,'assistant');
  const services={cwd,agentDir:cwd};
  const build=(sm,name)=>({name,sessionManager:sm,sessionFile:sm.getSessionFile(),model:{provider:'fixture',id:name==='initial'?'selected':'default'},thinkingLevel:name==='initial'?'high':'off',
   extensionRunner:{hasHandlers:()=>true,emit:async event=>{
    events.push({type:event.type,...(event.reason?{reason:event.reason}:{}),...(event.entryId?{entry:ids.get(event.entryId)??'unknown'}:{}),...(event.targetSessionFile?{target:files.get(event.targetSessionFile)??'unknown'}:{})});
    if(scenario.startsWith('cancel-'))return {cancel:true};
    if(scenario==='shutdown-failure'&&event.type==='session_shutdown')throw Error('owned shutdown failure');
   }},
   waitForEventLoop:async()=>events.push({type:'drain-events'}),
   dispose:()=>events.push({type:'dispose',session:name}),
   setModel:async function(model){this.model=model;sm.appendModelChange(model.provider,model.id);events.push({type:'set-model',model:model.id});},
   setThinkingLevel:function(level){this.thinkingLevel=level;sm.appendThinkingLevelChange(level);events.push({type:'set-thinking',level});},
  });
  const initial=build(manager,'initial');
  const factory=async options=>{
   files.set(options.sessionManager.getSessionFile(),files.get(options.sessionManager.getSessionFile())??'next');
   events.push({type:'prepare',reason:options.sessionStartEvent.reason});
   if(scenario==='prepare-failure')throw Error('owned preparation failure');
   return {session:build(options.sessionManager,'next'),services,diagnostics:[]};
  };
  const runtime=new AgentSessionRuntime(initial,services,factory);
  let result=null,error=null;
  try {
   if(scenario==='resume') {
    const other=SessionManager.create(cwd,dir);files.set(other.getSessionFile(),'other');
    other.appendMessage(message('user','Resumed instruction'));other.appendMessage(message('assistant','Resumed answer'));
    result=await runtime.switchSession(other.getSessionFile());
   } else if(scenario.includes('fork')) result=await runtime.fork(scenario==='fork-first'?first:scenario==='invalid-fork'?answer:later);
   else result=await runtime.newSession();
  } catch(e){error=e.message;}
  const branch=runtime.session.sessionManager.getBranch().map(entry=>entry.type==='message'?{type:entry.type,message:entry.message}:entry.type==='model_change'?{type:entry.type,provider:entry.provider,modelId:entry.modelId}:entry.type==='thinking_level_change'?{type:entry.type,thinkingLevel:entry.thinkingLevel}:{type:entry.type});
  rows.push({scenario,events,result,error,selected:runtime.session.name,branch,model:runtime.session.model.id,thinking:runtime.session.thinkingLevel,sourceChangedProcessCwd:process.cwd()!==priorCwd});
 } finally {process.chdir(priorCwd);fs.rmSync(cwd,{recursive:true,force:true});}
}
assert.equal(rows.length,9);
for(const row of rows) {
 assert.equal(row.selected,['new','resume','fork-first','fork-later'].includes(row.scenario)?'next':'initial');
 assert.equal(row.error!==null,['prepare-failure','shutdown-failure','invalid-fork'].includes(row.scenario));
 if(row.scenario.startsWith('cancel-'))assert.equal(row.result.cancelled,true);
}
assert.deepEqual(rows.find(row=>row.scenario==='new').branch.map(entry=>entry.type),['model_change','thinking_level_change']);
assert.equal(rows.find(row=>row.scenario==='new').model,'selected');
assert.equal(rows.find(row=>row.scenario==='new').thinking,'high');
assert.equal(rows.find(row=>row.scenario==='fork-first').branch.length,0);
assert.equal(rows.find(row=>row.scenario==='fork-later').result.selectedText,'Secondinstruction');
const sourceFiles=['packages/coding-agent/src/core/agent-session-runtime.ts','packages/coding-agent/src/core/session-manager.ts','packages/coding-agent/src/core/extensions/runner.ts'];
const captured={version:1,source_commit:'e30f0b4b238ee5618928bcd17b4571eb046e7038',source_files:sourceFiles.map(file=>({path:file,sha256:createHash('sha256').update(fs.readFileSync(path.join(base,file))).digest('hex')})),rows,model_actions:0,network_actions:0};
const target=path.join(root,'fixtures/runtime/session-lifecycle-oracle.json');
if(process.argv.includes('--capture')){assert(!fs.existsSync(target));fs.writeFileSync(target,JSON.stringify(captured,null,2)+'\n');}
else assert.deepEqual(captured,JSON.parse(fs.readFileSync(target,'utf8')));
console.log(JSON.stringify({status:'passed',cases:rows.length,model_actions:0,network_actions:0}));
