// Real pinned print coordinator, bounded scripted session callbacks, no inference.
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {fileURLToPath,pathToFileURL} from 'node:url';
await import('../offline_guard.mjs');
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../..');
const base=path.join(root,'reference/ag1-executable/packages/coding-agent');
const {runPrintMode}=await import(pathToFileURL(path.join(base,'dist/modes/print-mode.js')));
const {classifyControlledTaskIntent}=await import(pathToFileURL(path.join(base,'dist/core/task-policy.js')));
const prompts=['Fix the bug.','Implement the requested change.','Create a responsive frontend.','Refactor across all files.','Explain how to fix this; do not modify code files.','Read-only postmortem analysis only: explain the bug.','Review only; never change repository files.','Do not use tools; describe how to fix it.','Run tests only.','Verify the result.','Research current pricing.','Hello.','Which file is this?','Change the code.','Just inspect the bug, do not act.','',"Don't write files; explain the patch."];
const intents=prompts.map(prompt=>({prompt,intent:classifyControlledTaskIntent(prompt),mutationExpected:['small_patch','debug_fix','multi_file_change','frontend_visual'].includes(classifyControlledTaskIntent(prompt))}));
const assistant=(text,content)=>({role:'assistant',content:content??[{type:'text',text}],api:'openai-completions',provider:'owned-fixture',model:'fixture',usage:{input:0,output:0,cacheRead:0,cacheWrite:0,totalTokens:0,cost:{input:0,output:0,cacheRead:0,cacheWrite:0,total:0}},stopReason:content?'toolUse':'stop',timestamp:1000});
const result=(id,name,text)=>({role:'toolResult',toolCallId:id,toolName:name,isError:false,content:[{type:'text',text}],timestamp:1000});
const cases=[
 ['promises','Explain the source.',[[assistant('Let me inspect it:')],[assistant('Let me inspect it:')],[assistant('Done')]]],
 ['unverified','Implement the requested change.',Array.from({length:4},()=>[assistant('Done')])],
 ['verified','Fix the output file and run tests.',[[assistant('Done')],[assistant('',[{type:'toolCall',id:'write',name:'write',arguments:{path:'answer.txt',content:'42'}}]),result('write','write','Wrote answer.txt'),assistant('',[{type:'toolCall',id:'verify',name:'bash',arguments:{command:'node --test verifier.test.js'}}]),result('verify','bash','1 passed'),assistant('Done')]]],
];
const rows=[];
for(const [id,prompt,turns] of cases){
 const cwd=fs.mkdtempSync(path.join(os.tmpdir(),'ag1-print-oracle-'));
 const subscribers=[];const custom=[];const pending=[];let turn=0;let shutdowns=0;let aborts=0;
 const state={messages:[]};
 const emit=event=>{for(const callback of [...subscribers])callback(event)};
 const drive=()=>{
  assert(turn<turns.length,'unexpected SDK follow-up');
  emit({type:'agent_start'});
  for(const message of turns[turn++]){
   state.messages.push(message);
   emit({type:'message_end',message});
   if(message.role==='toolResult')emit({type:'tool_execution_end',toolName:message.toolName,isError:false,result:message});
  }
  emit({type:'agent_end'});
 };
 const session={
  sessionManager:{getHeader:()=>undefined,getCwd:()=>cwd,getSessionName:()=>undefined,getSessionId:()=>id,getEntries:()=>[]},
  agent:{waitForIdle:async()=>{}},state,
  extensionRunner:{hasHandlers:event=>event==='session_shutdown',emit:async event=>{if(event.type==='session_shutdown')shutdowns++}},
  bindExtensions:async()=>{},subscribe:callback=>{subscribers.push(callback);return()=>subscribers.splice(subscribers.indexOf(callback),1)},
  prompt:async text=>{state.messages.push({role:'user',content:text,timestamp:1000});drive()},
  sendCustomMessage:async(message,options)=>{
   custom.push({message,options});
   state.messages.push({role:'custom',...message,timestamp:1000});
   const timer=setTimeout(drive,1);pending.push(timer);
  },
  abort:async()=>{aborts++},reload:async()=>{},
  getActiveToolNames:()=>['read','write','bash'],setActiveToolsByName:()=>{},
  model:{provider:'owned-fixture',id:'fixture'},contextWindow:32768,thinkingLevel:'off',
 };
 const runtime={session,newSession:async()=>{},fork:async()=>({selectedText:''}),switchSession:async()=>({cancelled:false}),dispose:async()=>session.extensionRunner.emit({type:'session_shutdown'})};
 try {
  const exitCode=await runPrintMode(runtime,{mode:'text',initialMessage:prompt});
  assert.equal(turn,turns.length);
  rows.push({id,prompt,exitCode,turns:turn,custom,shutdowns,aborts});
 }finally{for(const timer of pending)clearTimeout(timer);fs.rmSync(cwd,{recursive:true,force:true});}
}
const sources=['src/modes/print-mode.ts','src/core/task-policy.ts'];
const captured=JSON.parse(JSON.stringify({version:1,source_commit:'e30f0b4b238ee5618928bcd17b4571eb046e7038',source_files:sources.map(file=>({path:'packages/coding-agent/'+file,sha256:createHash('sha256').update(fs.readFileSync(path.join(base,file))).digest('hex')})),intents,rows,model_actions:0,network_actions:0}));
const target=path.join(root,'fixtures/runtime/print-oracle.json');
if(process.argv.includes('--capture')){assert(!fs.existsSync(target));fs.writeFileSync(target,JSON.stringify(captured,null,2)+'\n');}
else assert.deepEqual(captured,JSON.parse(fs.readFileSync(target,'utf8')));
console.log(JSON.stringify({status:'passed',intents:intents.length,print_cases:rows.length,model_actions:0,network_actions:0}));
