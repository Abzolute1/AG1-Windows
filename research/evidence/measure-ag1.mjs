// Research harness only. Runs rebuilt reference modules against disposable data.
import fs, { mkdirSync, mkdtempSync, writeFileSync, readFileSync, readdirSync } from 'node:fs';
import { syncBuiltinESMExports } from 'node:module';
import { resolve, join } from 'node:path';
import { pathToFileURL } from 'node:url';
import { execFileSync } from 'node:child_process';
import os from 'node:os';
const root = resolve(process.argv[2] || 'research/scratch/ag1-reference');
const output = resolve(process.argv[3] || 'research/evidence/ag1-microbench.json');
const scratch = mkdtempSync(resolve('research/scratch/measure-'));
const imp = p => import(pathToFileURL(join(root,p)));
const { ArtifactStore } = await imp('packages/coding-agent/dist/core/artifact-store.js');
const { compileContextWorkingSet } = await imp('packages/coding-agent/dist/core/context-compiler.js');
const { SessionManager } = await imp('packages/coding-agent/dist/core/session-manager.js');
const { createSourceContextToolDefinition } = await imp('packages/coding-agent/dist/core/tools/source-context.js');
const { buildWorkspaceIndex, clearWorkspaceIndexCache } = await imp('packages/coding-agent/dist/core/workspace-index.js');
const { createSourceRevision, SourceRevisionValidator } = await imp('packages/coding-agent/dist/core/source-revision.js');
const { acquireInferenceSlot } = await imp('packages/coding-agent/dist/core/inference-scheduler.js');
const { createReadTool } = await imp('packages/coding-agent/dist/core/tools/read.js');
const { EventStream, AssistantMessageEventStream } = await imp('packages/ai/dist/utils/event-stream.js');
const { runAgentLoop } = await imp('packages/agent/dist/agent-loop.js');
const memory = await imp('packages/coding-agent/dist/core/memory-store.js');
const results = {};
async function bench(name, fn, n=30, warmup=3) {
  for(let i=0;i<warmup;i++) await fn();
  const times=[];
  for(let i=0;i<n;i++){const t=performance.now(); await fn(); times.push(performance.now()-t);}
  const sorted=[...times].sort((a,b)=>a-b);
  results[name]={unit:'ms',n,warmup,p50:sorted[Math.floor(n*.5)],p95:sorted[Math.min(n-1,Math.ceil(n*.95)-1)],min:sorted[0],max:sorted.at(-1),samples:times};
}
mkdirSync(join(scratch,'src'));
for(let i=0;i<120;i++)writeFileSync(join(scratch,'src',`module-${i}.ts`),`export function exactSymbol${i}(x: number) { return x + ${i}; }\n`.repeat(80));
execFileSync('git',['init','-q'],{cwd:scratch});
execFileSync('git',['add','src'],{cwd:scratch});
execFileSync('git',['-c','user.name=Research','-c','user.email=research@localhost','-c','commit.gpgsign=false','commit','-qm','fixture'],{cwd:scratch});
const store=new ArtifactStore(join(scratch,'artifacts'),'research');
const text='research immutable evidence marker α\n'.repeat(26000);
const record=store.putText(text,{producer:{type:'system',name:'measurement'}});
await bench('artifact_put_988000_bytes_deduplicated_blob_new_record',()=>store.putText(text,{producer:{type:'system',name:'measurement'}}));
await bench('artifact_get',()=>store.get(record.id));
await bench('artifact_read_8192_bytes',()=>store.read(record.id,0,8192));
await bench('artifact_search_marker',()=>store.search('marker'));
const session=SessionManager.create(scratch,join(scratch,'sessions'));
let i=0;
await bench('session_append_custom_async',()=>session.appendCustomEntryAsync('research',{i:i++,text:'state'.repeat(100)}));
await bench('session_reopen',()=>SessionManager.open(session.getSessionFile()));
await bench('workspace_index_uncached_120_files',()=>{clearWorkspaceIndexCache(scratch);return buildWorkspaceIndex({rootDir:scratch,maxFiles:5000,useCache:false});});
const source=createSourceContextToolDefinition(scratch,{artifactStore:store,sessionManager:session});
await bench('source_context_exact_symbol',()=>source.execute(`measure-${i++}`,{query:'exactSymbol42',symbols:['exactSymbol42']},undefined,()=>{}));
const revision=createSourceRevision(scratch,'src/module-42.ts');
const validator=new SourceRevisionValidator(scratch);
await bench('source_revision_hash',()=>createSourceRevision(scratch,'src/module-42.ts'));
await bench('source_revision_warm_validation',()=>validator.validate(revision.reference));
await bench('git_status',()=>execFileSync('git',['status','--porcelain=v1','-z'],{cwd:scratch}));
await bench('git_diff',()=>execFileSync('git',['diff','--no-ext-diff','--','src'],{cwd:scratch}));
const model={provider:'llamacpp',id:'research',api:'openai-completions',contextWindow:32768,maxTokens:4096};
await bench('scheduler_uncontended',async()=>{const release=await acquireInferenceSlot(model);release();},100);
const read=createReadTool(scratch);
await bench('read_tool_dispatch',()=>read.execute(`r-${i++}`,{path:'src/module-42.ts'},undefined,()=>{}));
const usage={input:0,output:0,cacheRead:0,cacheWrite:0,totalTokens:0,cost:{input:0,output:0,cacheRead:0,cacheWrite:0,total:0}};
const assistant=(content,stopReason='stop')=>({role:'assistant',content,api:model.api,provider:model.provider,model:model.id,usage,stopReason,timestamp:i++});
const history=[{role:'user',content:'Find the exact evidence',timestamp:0}];
for(let r=0;r<100;r++){history.push(assistant([{type:'toolCall',id:`call-${r}`,name:'read',arguments:{path:`src/module-${r}.ts`}}],'toolUse'));history.push({role:'toolResult',toolCallId:`call-${r}`,toolName:'read',content:[{type:'text',text:'fixture evidence '.repeat(1500)}],isError:false,timestamp:r});}
await bench('context_compile_201_messages_2550000_text_chars',()=>compileContextWorkingSet(structuredClone(history),{artifactStore:store,contextWindow:32768,fixedTokens:1200}),10,1);
const compiled=compileContextWorkingSet(structuredClone(history),{artifactStore:store,contextWindow:32768,fixedTokens:1200});
await bench('context_compile_already_compiled',()=>compileContextWorkingSet(compiled.messages,{artifactStore:store,contextWindow:32768,fixedTokens:1200}));
await bench('event_stream_10000_events',async()=>{const s=new EventStream(e=>e===10000,e=>e);for(let j=0;j<=10000;j++)s.push(j);s.end();for await(const _ of s){};},10);
// Reproduce aggregate tool-result limit violation without AgentSession externalization.
let calls=0;const events=[];
const streamFn=()=>{const s=new AssistantMessageEventStream();const msg=assistant(calls++===0?[{type:'toolCall',id:'probe',name:'probe',arguments:{}}]:[{type:'text',text:'done'}],calls===1?'toolUse':'stop');s.push({type:'done',reason:msg.stopReason,message:msg});s.end();return s;};
const msgs=await runAgentLoop([{role:'user',content:'probe',timestamp:0}],{systemPrompt:'',messages:[],tools:[{name:'probe',label:'probe',description:'probe',parameters:{type:'object',properties:{}},execute:async()=>({content:[{type:'text',text:'a'.repeat(300)},{type:'text',text:'b'.repeat(300)}],details:{}})}]},{model,convertToLlm:m=>m,maxToolResultTokens:100},e=>events.push(e),undefined,streamFn);
const tool=msgs.find(m=>m.role==='toolResult');
const memPath=memory.memoryStorePath(scratch);mkdirSync(resolve(memPath,'..'),{recursive:true});writeFileSync(memPath,'{"version":3,"entries":[');
const corruptLoad=memory.loadMemoryStore(scratch);memory.updateMemoryStore(scratch,s=>s);
const probes={aggregateToolCap:{limitTokens:100,limitChars:400,actualTextChars:tool.content.reduce((n,b)=>n+(b.text?.length||0),0)},corruptMemory:{loadReturnedEntries:corruptLoad.entries.length,afterUpdate:JSON.parse(readFileSync(memPath,'utf8')),quarantineCreated:readdirSync(resolve(memPath,'..')).some(p=>p.includes('corrupt')||p.includes('quarantine'))}};
// Inject an external edit between source text read and the separate revision read.
const racePath=join(scratch,'src/race.ts');writeFileSync(racePath,'export const raceValue = "OLD_EVIDENCE";\n');
const originalRead=fs.readFileSync;let injected=false;let raceResult;
fs.readFileSync=function(p,...args){const value=originalRead(p,...args);if(String(p)===racePath&&!injected){injected=true;writeFileSync(racePath,'export const raceValue = "NEW_EVIDENCE";\n');}return value;};
syncBuiltinESMExports();
try{raceResult=await source.execute('race-probe',{query:'raceValue',paths:['src/race.ts']},undefined,()=>{});}finally{fs.readFileSync=originalRead;syncBuiltinESMExports();}
const raceRef=raceResult.details.files[0].reference;
probes.sourceReadRevisionRace={injected,returnedOldEvidence:raceResult.content.some(b=>b.text?.includes('OLD_EVIDENCE')),currentFileContainsNew:readFileSync(racePath,'utf8').includes('NEW_EVIDENCE'),claimedRevisionStatus:validator.validate(raceRef).status};
const cpuBefore=process.cpuUsage();const idleStart=performance.now();await new Promise(r=>setTimeout(r,2000));const cpu=process.cpuUsage(cpuBefore);
const data={date:new Date().toISOString(),node:process.version,platform:process.platform,cpu:os.cpus()[0].model,totalMemory:os.totalmem(),scratch,source:root,modelCalls:0,notes:['Synthetic microbenchmarks; not end-to-end production latency.','Measurements sequential, hot filesystem, no CPU pinning.','Artifact put benchmark reuses content but creates records.','Context benchmark includes structuredClone and growing artifact catalog.','Idle sample is post-benchmark harness, not full AG1 app.'],results,compiler:compiled.stats,probes,idleHarness:{wallMs:performance.now()-idleStart,cpuMicroseconds:cpu,memory:process.memoryUsage()}};
writeFileSync(output,JSON.stringify(data,null,2)+'\n');console.log(JSON.stringify({output,probes,metrics:Object.fromEntries(Object.entries(results).map(([k,v])=>[k,{p50:v.p50,p95:v.p95}]))},null,2));
