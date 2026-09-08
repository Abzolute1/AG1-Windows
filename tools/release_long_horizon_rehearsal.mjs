// Offline composition check. Scripted answers never establish live quality.
import fs from 'node:fs';
import path from 'node:path';
import assert from 'node:assert/strict';
import crypto from 'node:crypto';
import {pathToFileURL} from 'node:url';
const root=process.cwd(), [preparedArg, seedArg]=process.argv.slice(2);
assert(preparedArg && seedArg, 'Prepared directory and seed report required');
const prepared=fs.realpathSync(preparedArg), seedPath=fs.realpathSync(seedArg);
for(const p of [prepared,seedPath]) assert(p.startsWith(fs.realpathSync(path.join(root,'.local'))+path.sep));
const hash=s=>crypto.createHash('sha256').update(s).digest('hex');
const preparation=JSON.parse(fs.readFileSync(path.join(prepared,'preparation.json')));
assert.equal(hash(fs.readFileSync(preparation.source)),preparation.source_sha256);
assert.equal(hash(fs.readFileSync(path.join(prepared,'fixture-generator.mjs'))),preparation.generated_sha256);
const seed=JSON.parse(fs.readFileSync(seedPath));
assert(fs.realpathSync(seed.source).startsWith(fs.realpathSync(path.join(root,'.local'))+path.sep));
const out=path.join(prepared,'ts-rehearsal');fs.mkdirSync(out,{mode:0o700});
const core=path.join(root,'reference/ag1-executable/packages/coding-agent/dist/core');
const load=name=>import(pathToFileURL(path.join(core,name+'.js')));
const [{createAgentSession},{AuthStorage},{ModelRegistry},{SettingsManager},{SessionManager},{createExtensionRuntime},{ArtifactStore},{externalizeLargeToolResult}]=await Promise.all(['sdk','auth-storage','model-registry','settings-manager','session-manager','extensions/index','artifact-store','tool-result-artifacts'].map(load));
const {AssistantMessageEventStream}=await import(pathToFileURL(path.join(root,'reference/ag1-executable/packages/ai/dist/utils/event-stream.js')));
const {createBenchmark}=await import(pathToFileURL(path.join(prepared,'fixture-generator.mjs')));
const auth=AuthStorage.inMemory(), registry=ModelRegistry.inMemory(auth);
const model={id:'p14-scripted',name:'Offline rehearsal',api:'openai-completions',provider:'p14-fixture',baseUrl:'http://127.0.0.1:1/v1',reasoning:false,input:['text'],cost:{input:0,output:0,cacheRead:0,cacheWrite:0},contextWindow:32768,maxTokens:2048};
registry.registerProvider(model.provider,{api:model.api,baseUrl:model.baseUrl,apiKey:'offline-fixture',models:[model]});
let currentSpec, calls;
const requests=[];
const fixture=createBenchmark({ArtifactStore,SessionManager,SettingsManager,externalizeLargeToolResult,createExtensionRuntime,
  fetch(){throw new Error('No runtime owner: offline rehearsal forbids tokenization/network');},
  async createAgentSession(options){
    const result=await createAgentSession(options);
    const spec=currentSpec;
    const fixed=spec.id==='recovery'?'export function recoveryKey(service, attempt) { return String(service).toLowerCase() + "|" + attempt; }\n':spec.initialSource;
    const actions=[['artifact',{action:'search',query:spec.marker}],['read',{path:spec.sourcePath}],['edit',{path:spec.sourcePath,oldText:spec.brokenSource,newText:fixed}],['bash',{command:`node ${spec.verification}`}]];
    result.session.agent.streamFn=(_model,context)=>{
      assert(calls<5,'Unexpected extra generation');
      requests.push({checkpoint:spec.id,call:calls,context:JSON.parse(JSON.stringify(context))});
      const action=actions[calls++], stream=new AssistantMessageEventStream();
      const content=action?[{type:'toolCall',id:crypto.randomUUID(),name:action[0],arguments:action[1]}]:[{type:'text',text:'Scripted rehearsal complete.'}];
      const stopReason=action?'toolUse':'stop';
      const message={role:'assistant',content,api:model.api,provider:model.provider,model:model.id,usage:{input:0,output:0,cacheRead:0,cacheWrite:0,totalTokens:0,cost:{}},timestamp:Date.now(),stopReason};
      queueMicrotask(()=>{stream.push({type:'done',reason:stopReason,message});stream.end();});return stream;
    };
    return result;
  }});
const workspace=path.join(out,'workspace'), state=path.join(out,'state'), agentDir=path.join(out,'agent');
await fixture.createWorkspace(workspace);fs.cpSync(seed.source,state,{recursive:true});fs.mkdirSync(agentDir,{mode:0o700});
const checked=async(args)=>{const r=await fixture.runCommand('git',args,{cwd:workspace});assert.equal(r.code,0,r.stderr);};
const results=[];
for(const [index,spec] of fixture.CHECKPOINT_SPECS.entries()){
  currentSpec=spec;calls=0;
  fs.writeFileSync(path.join(workspace,spec.sourcePath),spec.brokenSource);
  await checked(['add',spec.sourcePath]);await checked(['commit','-q','-m',`introduce ${spec.id} regression`]);
  const manager=SessionManager.open(path.join(state,path.basename(seed.journal)),state,workspace);
  const evidence=seed.artifacts.find(a=>a.marker===spec.marker);assert(evidence);
  const result=await fixture.executeCheckpoint({id:spec.id,task:fixture.checkpointTask(index,spec,evidence),marker:spec.marker,verification:spec.verification,allowedFiles:[spec.sourcePath],sessionManager:manager,workspace,agentDir,model,registry,auth,thinking:'off',timeoutSeconds:60,logRoot:path.join(out,'logs')});
  results.push(result);
  fs.writeFileSync(path.join(out,'checkpoints.json'),JSON.stringify(results,null,2));
  assert.equal(calls,5);assert(result.verificationPassed && result.evidenceRehydrated && !result.error,JSON.stringify(result));
  assert.equal(result.context.latest.overTargetTokens,0);
  for(const verifier of fixture.CHECKPOINT_SPECS) assert.equal(fs.readFileSync(path.join(workspace,verifier.verification),'utf8'),verifier.verificationSource);
  await checked(['add',spec.sourcePath]);await checked(['commit','-q','-m',`checkpoint ${spec.id}`]);
}
fs.writeFileSync(path.join(out,'requests.json'),JSON.stringify(requests,null,2),{flag:'wx'});
fs.writeFileSync(path.join(out,'result.json'),JSON.stringify({status:'passed-offline-preparation',fixture:true,release_acceptance:false,live_model_actions:0,scripted_provider_calls:requests.length,checkpoint_count:results.length,original_checkpoint_verifier:true,original_task_expression:true,exact_token_calibration:false,scope:'Six original checkpoint evaluators driven by scripted answers through real TS sessions, reopening the same seeded history. No live quality, duration or autonomous task claim.'},null,2),{flag:'wx'});
console.log('Six original checkpoint verifiers passed through real TS sessions with scripted responses and journal reopening.');
