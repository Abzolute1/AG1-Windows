// Calls the pinned resolver/provider with fake process/HTTP dependencies only.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import { join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
const root = resolve(process.argv[2]);
const destination = resolve(process.argv[3]);
const lock = JSON.parse(fs.readFileSync(resolve('planning/evidence/llama-runtime-lock.json'), 'utf8'));
const runtime = await import(pathToFileURL(join(root, 'packages/coding-agent/dist/core/llamacpp-runtime.js')));
const {streamOpenAICompletions} = await import(pathToFileURL(join(root, 'packages/ai/dist/providers/openai-completions.js')));
const scratch = fs.mkdtempSync(join(process.env.TMPDIR || os.tmpdir(), 'ag1-launch-capture-'));
const profiles = Object.fromEntries(Object.entries(lock.models).map(([id, row]) => [id, row.profile]));
const captures = [];
try {
  for (const [id, profile] of Object.entries(profiles)) {
    for (const scenario of ['profile-default', 'context-override-90001', 'env-moe-override']) {
      const dir = join(scratch, String(captures.length));fs.mkdirSync(dir);
      const fakeModel = join(dir, id + '.gguf');fs.writeFileSync(fakeModel, 'GGUF fixture; never load');
      fs.writeFileSync(runtime.modelProfilesPath(dir), JSON.stringify({version:1, models:profiles}));
      const spawnCalls = []; let fakeHealthCalls = 0;
      const env = {AG1_LLAMA_CPP:'1', AG1_LLAMA_SERVER:lock.environment_path_overrides.AG1_LLAMA_SERVER,
        AG1_LLAMA_LIB_PATH:lock.environment_path_overrides.AG1_LLAMA_LIB_PATH,
        AG1_LLAMA_CPP_PORT:'18080', LD_LIBRARY_PATH:'/fixture/inherited-libs'};
      if (scenario === 'env-moe-override') env.AG1_LLAMA_N_CPU_MOE='7';
      const outcome = await runtime.ensureLlamaCppServer({
        agentDir:dir, env, searchDirs:[dir], preferredModelId:id, waitReadyMs:0, vramTune:false,
        ...(scenario === 'context-override-90001' ? {contextWindow:90001, useModelProfileContext:false} : {}),
        deps:{env, homedir:dir, vramMiB:24576, now:()=>1000,
          fetchImpl:async()=>{fakeHealthCalls++;return new Response('',{status:503});},
          spawnImpl:(command,args,options)=>{spawnCalls.push({command,args,environment:options.env});return {pid:987654,unref(){}};},
          pidAlive:()=>false, listListenPids:()=>[], kill:()=>{throw new Error('Unexpected kill');},
          sleep:async()=>{throw new Error('Unexpected readiness/tuning sleep');}},
      });
      assert.equal(spawnCalls.length,1);assert.equal(spawnCalls[0].command,profile.serverPath);
      assert.equal(outcome.started,true);assert.equal(outcome.ready,false);
      const launch=spawnCalls[0];const value=flag=>launch.args[launch.args.indexOf(flag)+1];
      assert.deepEqual(launch.args.slice(-profile.extraArgs.length), profile.extraArgs);
      assert.equal(launch.args.includes('--prio'),profile.priority!==false);
      if(profile.slotSimilarityFlag==='none') assert(!launch.args.some(x=>x.startsWith('--slot-')));
      if(profile.slotSimilarityFlag==='prompt') assert.equal(value('--slot-prompt-similarity'),'0');
      if(id.includes('Flash')&&scenario==='profile-default')assert.equal(value('--ubatch-size'),'1024');
      if(id.includes('Flash')&&scenario==='context-override-90001')assert.equal(value('--ubatch-size'),'512');
      if(scenario==='env-moe-override')assert.equal(value('--n-cpu-moe'),'7');
      const normalize=x=>typeof x==='string'?x.replaceAll(dir,'<fixture>'):x;
      captures.push({id:`${id}/${scenario}`,model:id,scenario,
        inputs:{profile,contextOverride:scenario==='context-override-90001'?90001:null,fixtureVramMiB:24576,environment:env},
        observed:{command:launch.command,args:launch.args.map(normalize),environment:launch.environment,
          providerConfig:outcome.providerConfig,ready:false,fakeHealthCalls},
        normalization:['Temporary model/agent directory replaced with <fixture>; model IDs, argument order and settings unchanged.']});
    }
  }
  // Explicitly distinguish missing/false/none from the reference's defaults.
  const base = {modelPath:'/fixture/model.gguf',alias:'fixture',host:'127.0.0.1',port:18080,contextWindow:32768,decodeThreads:12,batchThreads:24};
  const defaults = runtime.buildLlamaCppServerArgs(base);
  assert(defaults.includes('--slot-cache-key-similarity'));
  const disabled = runtime.buildLlamaCppServerArgs({...base,priority:false,slotSimilarityFlag:'none',kvUnified:false,flashAttention:false});
  assert(!disabled.includes('--prio'));assert(!disabled.includes('--kv-unified'));assert(!disabled.includes('--slot-cache-key-similarity'));
  const payloads=[];
  for(const [id,profile] of Object.entries(profiles)){
    const config=runtime.buildLlamaCppProviderConfig('http://127.0.0.1:18080',[{id,name:id,path:'/fixture/'+id+'.gguf',sizeBytes:1,reasoning:true}],profile.contextWindow,{profiles,activeModelId:id,activeContextWindow:profile.contextWindow,activeVision:false});
    const model={...config.models[0],provider:'llamacpp',api:config.api,baseUrl:config.baseUrl};
    for(const reasoning of [undefined,'high']){
      let payload;
      const stream=streamOpenAICompletions(model,{systemPrompt:'Fixture system',messages:[{role:'user',content:'Return a tool call or answer.',timestamp:0}],tools:[{name:'read',description:'Fixture read',parameters:{type:'object',properties:{path:{type:'string'}},required:['path']}}]},
        {apiKey:'fixture-no-credential',reasoningEffort:reasoning,maxTokens:256,onPayload:p=>{payload=structuredClone(p);throw new Error('P01_PAYLOAD_CAPTURE_ONLY');}});
      const result=await stream.result();
      assert(payload);assert.equal(result.stopReason,'error');assert(result.errorMessage.includes('P01_PAYLOAD_CAPTURE_ONLY'));
      assert.equal(payload.cache_prompt,true);assert.equal(payload.temperature,0.7);
      payloads.push({model:id,reasoning:reasoning??'off',input:{maxTokens:256},payload,networkRequests:0});
    }
  }
  fs.mkdirSync(destination,{recursive:true});
  fs.writeFileSync(join(destination,'launch-plans.json'),JSON.stringify({version:1,sourceCommit:'e30f0b4b238ee5618928bcd17b4571eb046e7038',captures,edgeCases:{defaults,disabled},modelActions:{spawn:0,kill:0,network:0,tune:0},scope:'Real pinned TS resolver with fake process/health; fixture env/hardware explicitly recorded, not a live deployment manifest.'},null,2)+'\n');
  fs.writeFileSync(join(destination,'request-payloads.json'),JSON.stringify({version:1,payloads,scope:'Real buildParams reached through provider onPayload hook; sentinel stops before transport.'},null,2)+'\n');
  console.log(JSON.stringify({launchPlans:captures.length,requestPayloads:payloads.length,modelActions:0}));
} finally {fs.rmSync(scratch,{recursive:true,force:true});}
