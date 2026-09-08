// Supervised TS preflight. Runtime ownership stays in the Rust parent.
import fs from 'node:fs';
import path from 'node:path';
import net from 'node:net';
import crypto from 'node:crypto';
import assert from 'node:assert/strict';
import {pathToFileURL} from 'node:url';
const input=JSON.parse(fs.readFileSync(process.argv[2],'utf8'));
assert.equal(typeof input.fixture,'boolean');
const {model,workload,source,destination,control}=input;
assert.equal(model.provider,'llamacpp');assert.equal(workload.mode,'seed-and-typescript');
const root=fs.realpathSync(destination);
const hash=bytes=>crypto.createHash('sha256').update(bytes).digest('hex');
const save=(name,value)=>fs.writeFileSync(path.join(root,name),JSON.stringify(value,null,2)+'\n',{flag:'wx',mode:0o600});
const prepare=()=>new Promise((resolve,reject)=>{
  const socket=net.createConnection(control.socket);let response='';
  socket.setTimeout(950000,()=>socket.destroy(new Error('Native owner preparation timed out')));
  socket.on('connect',()=>socket.write(JSON.stringify({key:control.key,action:'prepare',model:model.id})+'\n'));
  socket.on('error',reject);
  socket.on('data',chunk=>{response+=chunk;if(response.length>4096)socket.destroy(new Error('Owner response too large'));});
  socket.on('end',()=>{try{assert.equal(JSON.parse(response).prepared,true,'Native owner refused preparation');resolve();}catch(error){reject(error);}});
});
const core=path.join(source,'packages/coding-agent/dist/core');
const load=name=>import(pathToFileURL(path.join(core,name+'.js')));
const [{createAgentSession},{AuthStorage},{ModelRegistry},{SettingsManager},{SessionManager},{createExtensionRuntime},{ArtifactStore},{externalizeLargeToolResult}]=await Promise.all(['sdk','auth-storage','model-registry','settings-manager','session-manager','extensions/index','artifact-store','tool-result-artifacts'].map(load));
const generated=fs.realpathSync(workload.generated.path);assert.equal(hash(fs.readFileSync(generated)),workload.generated.sha256);
const {createBenchmark}=await import(pathToFileURL(generated));
const auth=AuthStorage.inMemory(),registry=ModelRegistry.inMemory(auth);
registry.registerProvider(model.provider,{api:model.api,baseUrl:model.baseUrl,apiKey:'p14-local-no-credential',models:[model]});
let requestCount=0,transportCount=0;
const benchmark=createBenchmark({ArtifactStore,SessionManager,SettingsManager,externalizeLargeToolResult,createExtensionRuntime,
  // The parent admitted and loaded this exact model before starting this child.
  // Only the selected tokenizer endpoint is available through this binding.
  async fetch(url,options){
    const expected=new URL(model.baseUrl);expected.pathname=expected.pathname.replace(/\/v1\/?$/,'/tokenize');expected.search='';expected.hash='';
    assert.equal(String(url),expected.toString());assert.equal(options.method,'POST');
    return globalThis.fetch(url,{...options,redirect:'error',signal:AbortSignal.timeout(120000)});
  },
  async createAgentSession(options){
    // P14 retains judge-only vision. The original coding fixture activates no
    // visual tools; suppress its unrelated background image-generation probe.
    // This settings manager is the original evaluator's in-memory instance.
    options.settingsManager.setVisualToolsMode('off');
    const result=await createAgentSession(options);
    const coordinator=result.session._modelCoordinator;
    assert.equal(typeof coordinator?.ensureLlamaCppAlive,'function','Pinned TS lifecycle changed');
    coordinator.ensureLlamaCppAlive=prepare;
    // The original prompt path catches lifecycle errors. Check again at the
    // actual provider boundary so lost parent custody cannot admit a request.
    const stream=result.session.agent.streamFn;
    assert.equal(typeof stream,'function');
    result.session.agent.streamFn=async (...args)=>{
      await prepare();
      const selected=args[0];assert.equal(selected.id,model.id);assert.equal(selected.provider,model.provider);assert.equal(selected.baseUrl,model.baseUrl);
      const serial=++requestCount;
      save(`request-${String(serial).padStart(4,'0')}.json`,{model:selected,context:JSON.parse(JSON.stringify(args[1])),options:JSON.parse(JSON.stringify(args[2]??{},(key,value)=>key==='apiKey'?undefined:value))});
      const expectedReasoning=model.reasoning?'low':undefined;
      assert.equal(args[2]?.reasoning,expectedReasoning,'Selected TS thinking level differs from the frozen workload');
      const previous=args[2]?.onPayload;
      args[2]={...args[2],onPayload:async(payload,selectedModel)=>{
        const transformed=previous?await previous(payload,selectedModel):undefined;
        const effective=transformed===undefined?payload:transformed;
        save(`transport-${String(++transportCount).padStart(4,'0')}.json`,{request:serial,body:effective});
        if(model.reasoning)assert.equal(effective.chat_template_kwargs?.enable_thinking,true,'TS request disabled selected thinking');
        return transformed;
      }};
      return stream(...args);
    };
    return result;
  }});
const seedWorkspace=path.join(root,'seed-workspace'),seedState=path.join(root,'seed-state');
await prepare();
await benchmark.createWorkspace(seedWorkspace);
const manager=SessionManager.create(seedWorkspace,seedState);
const seeded=await benchmark.seedHistory({sessionManager:manager,model,rawTokens:workload.rawTokens,phases:workload.phases,checkpointSpecs:benchmark.CHECKPOINT_SPECS});
const initialStore=new ArtifactStore(benchmark.artifactRoot(manager),manager.getSessionId());
const intact=await benchmark.verifyArtifactSamples(initialStore,seeded.artifacts);
const withinTarget=seeded.totalTokens>=workload.rawTokens*.97 && seeded.totalTokens<=workload.rawTokens*1.03;
const seedReport={fixture:input.fixture,release_acceptance:false,workspace:seedWorkspace,source:seedState,session:manager.getSessionId(),journal:manager.getSessionFile(),model,token_count:seeded.totalTokens,token_counts_exact:seeded.tokenCountsExact,raw_token_target_met:withinTarget,artifacts_intact:intact,artifacts:seeded.artifacts};
save('seed.json',seedReport);
assert(seeded.tokenCountsExact && withinTarget && intact,'Exact live seed/token integrity requirements failed');
const workspace=path.join(root,'ts-workspace'),state=path.join(root,'ts-state'),agentDir=path.join(root,'ts-agent');
fs.cpSync(seedWorkspace,workspace,{recursive:true});fs.cpSync(seedState,state,{recursive:true});fs.mkdirSync(agentDir,{mode:0o700});
const git=async args=>{const result=await benchmark.runCommand('git',args,{cwd:workspace});assert.equal(result.code,0,result.stderr);};
const checkpoints=[];let activeManager;
for(const [index,spec] of benchmark.CHECKPOINT_SPECS.entries()){
  fs.writeFileSync(path.join(workspace,spec.sourcePath),spec.brokenSource);
  await git(['add',spec.sourcePath]);await git(['commit','-q','-m',`introduce ${spec.id} regression`]);
  activeManager=SessionManager.open(path.join(state,path.basename(manager.getSessionFile())),state,workspace);
  const evidence=benchmark.findMarkerArtifact(seeded.artifacts,spec.marker);
  const result=await benchmark.executeCheckpoint({id:index===0?spec.id:`${spec.id}-after-restart`,task:benchmark.checkpointTask(index,spec,evidence),marker:spec.marker,verification:spec.verification,allowedFiles:[spec.sourcePath],sessionManager:activeManager,workspace,agentDir,model,registry,auth,thinking:'low',timeoutSeconds:1200,logRoot:path.join(root,'ts-logs')});
  for(const verifier of benchmark.CHECKPOINT_SPECS) assert.equal(fs.readFileSync(path.join(workspace,verifier.verification),'utf8'),verifier.verificationSource);
  checkpoints.push(result);save(`checkpoint-${index+1}.json`,result);
  console.error(`P14 TS checkpoint=${result.id} verifier=${result.verificationPassed} recall=${result.evidenceRehydrated} error=${result.error??'none'}`);
  if(!result.verificationPassed || !result.evidenceRehydrated || result.error) break;
  await git(['add',spec.sourcePath]);await git(['commit','-q','-m',`checkpoint ${spec.id}`]);
}
const finalStore=new ArtifactStore(benchmark.artifactRoot(activeManager),activeManager.getSessionId());
const finalIntact=await benchmark.verifyArtifactSamples(finalStore,seeded.artifacts);
const activeWithinTarget=checkpoints.every(r=>r.context.latest && r.context.latest.overTargetTokens===0);
const passed=checkpoints.length===6 && checkpoints.every(r=>r.verificationPassed && r.evidenceRehydrated && !r.error) && finalIntact && activeWithinTarget;
save('typescript-result.json',{status:passed?'passed':'failed',fixture:input.fixture,release_acceptance:false,model,request_count:requestCount,checkpoints,active_within_target:activeWithinTarget,artifacts_intact_after:finalIntact,seed_sha256:hash(fs.readFileSync(path.join(root,'seed.json'))),vision_probe_disabled:true,scope:'Original six live long-horizon checkpoint evaluators; exclusive native runtime custody and exact externalized token counts. This preflight is not an 8h/24h duration result.'});
if(!passed)process.exitCode=1;
