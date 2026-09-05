// Full AgentSession fixture: preserve request projections separately from history.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import {join, resolve} from 'node:path';
import {pathToFileURL} from 'node:url';
const reference=resolve(process.argv[2]), output=resolve(process.argv[3]);
const imp=path=>import(pathToFileURL(join(reference,path)));
const {Agent}=await imp('packages/agent/dist/agent.js');
const {AssistantMessageEventStream}=await imp('packages/ai/dist/utils/event-stream.js');
const {AgentSession}=await imp('packages/coding-agent/dist/core/agent-session.js');
const {SessionManager}=await imp('packages/coding-agent/dist/core/session-manager.js');
const {SettingsManager}=await imp('packages/coding-agent/dist/core/settings-manager.js');
const {AuthStorage}=await imp('packages/coding-agent/dist/core/auth-storage.js');
const {ModelRegistry}=await imp('packages/coding-agent/dist/core/model-registry.js');
const {createExtensionRuntime}=await imp('packages/coding-agent/dist/core/extensions/loader.js');
const marker='[AG1 REQUEST-LOCAL WORKING STATE';
const usage={input:0,output:0,cacheRead:0,cacheWrite:0,totalTokens:0,cost:{input:0,output:0,cacheRead:0,cacheWrite:0,total:0}};
const model={provider:'fixture',id:'fixture',name:'Fixture',api:'openai-completions',baseUrl:'http://127.0.0.1:1',contextWindow:200000,maxTokens:4096,input:['text'],reasoning:false,cost:{input:0,output:0,cacheRead:0,cacheWrite:0}};
const scenarios=[];
for(const failure of [false,true]){
 const dir=fs.mkdtempSync(join(process.env.TMPDIR,'ag1-session-capture-'));
 const calls=[],events=[];let index=0,executions=0;
 const scripted=[[{type:'text',text:'answer one'}],[{type:'toolCall',id:'echo-call',name:'echo',arguments:{text:'round-2'}}],[{type:'text',text:'tool done'}],[{type:'text',text:'answer three'}]];
 const streamFn=(_model,context)=>{
  calls.push({systemPrompt:context.systemPrompt,messages:structuredClone(context.messages),tools:context.tools?.map(({name,description,parameters})=>({name,description,parameters:structuredClone(parameters)}))});
  const stream=new AssistantMessageEventStream(),content=scripted[index++];assert(content,'Unexpected extra inference');
  const message={role:'assistant',content,api:model.api,provider:model.provider,model:model.id,usage:structuredClone(usage),timestamp:0,stopReason:content[0].type==='toolCall'?'toolUse':'stop'};
  queueMicrotask(()=>{stream.push({type:'done',reason:message.stopReason,message});stream.end();});return stream;
 };
 const tool={name:'echo',label:'Echo',description:'Fixture echo',parameters:{type:'object',properties:{text:{type:'string'}},required:['text']},execute:async(_id,args)=>{executions++;if(failure)throw new Error('Fixture tool failure');return {content:[{type:'text',text:`echoed: ${args.text}`}],details:{}};}};
 const agent=new Agent({getApiKey:()=> 'fixture',initialState:{model,systemPrompt:'Fixture',tools:[tool]},streamFn});
 const manager=SessionManager.inMemory();const settings=SettingsManager.create(dir,dir);const auth=AuthStorage.create(join(dir,'auth.json'));auth.setRuntimeApiKey(model.provider,'fixture');
 const resources={getExtensions:()=>({extensions:[],errors:[],runtime:createExtensionRuntime()}),getSkills:()=>({skills:[],diagnostics:[]}),getPrompts:()=>({prompts:[],diagnostics:[]}),getThemes:()=>({themes:[],diagnostics:[]}),getAgentsFiles:()=>({agentsFiles:[]}),getSystemPrompt:()=>undefined,getAppendSystemPrompt:()=>[],extendResources(){},reload:async()=>{}};
 const session=new AgentSession({agent,sessionManager:manager,settingsManager:settings,cwd:dir,modelRegistry:ModelRegistry.create(auth,dir),resourceLoader:resources,baseToolsOverride:{echo:tool}});
 const unsub=session.subscribe(event=>events.push(structuredClone(event)));
 try{
  await session.prompt('first question');
  await session.prompt('use the echo tool',{systemPromptAddendum:'Fixture turn-only addendum'});
  await session.prompt('final question');
  assert.equal(calls.length,4);assert.equal(executions,1);
  const expectedInstructions=['first question','use the echo tool','use the echo tool','final question'];
  for(const [i,call] of calls.entries()){
   const carriers=call.messages.filter(m=>JSON.stringify(m.content).includes(marker));
   assert.equal(carriers.length,1,'Working state must have exactly one request-local carrier');
   assert.equal(carriers[0],call.messages.findLast(m=>m.role==='user'),'State belongs to current user turn');
   const text=JSON.stringify(carriers[0].content);
   assert(text.includes('objective: first question'));
   assert(text.includes('latest_user_instruction: '+expectedInstructions[i]));
  }
  assert.equal(calls[0].systemPrompt,calls[3].systemPrompt,'Turn addendum must not leak');
  assert.equal(calls[1].systemPrompt,calls[2].systemPrompt,'Tool-loop system prefix must remain stable');
  if(!failure)assert.deepEqual(calls[2].messages.slice(0,calls[1].messages.length),calls[1].messages,'Stable tool-loop prefix');
  assert(!JSON.stringify(agent.state.messages).includes(marker),'Projection must not enter canonical messages');
  assert(!JSON.stringify(manager.getBranch()).includes(marker),'Projection must not enter journal');
  const results=agent.state.messages.filter(m=>m.role==='toolResult');assert.equal(results.length,1);assert.equal(Boolean(results[0].isError),failure);
  const normalize=value=>JSON.parse(JSON.stringify(value, (key,value)=>key==='timestamp'?'<clock>':typeof value==='string'?value.replaceAll(dir,'<workspace>'):value));
  scenarios.push({id:failure?'session-tool-failure':'session-tool-success',inputs:{instructions:['first question','use the echo tool','final question'],toolThrows:failure,provider:'scripted fixture'},requests:normalize(calls),events:normalize(events),canonicalMessages:normalize(agent.state.messages),journal:normalize(manager.getBranch()),assertions:{singleRequestLocalCarrier:true,currentInstructionCorrect:true,originalObjectivePreserved:true,projectionAbsentFromJournal:true,toolExecutedOnce:true,errorOutcome:failure},normalization:['Timestamp values and disposable workspace path only; full working-state projection preserved.']});
 }finally{unsub();await session.dispose();fs.rmSync(dir,{recursive:true,force:true});}
}
fs.mkdirSync(resolve(output,'..'),{recursive:true});fs.writeFileSync(output,JSON.stringify({version:1,scenarios,modelActions:0},null,2)+'\n');
console.log(JSON.stringify({scenarios:scenarios.length,requests:scenarios.reduce((n,s)=>n+s.requests.length,0),modelActions:0}));
