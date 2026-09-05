import fs from 'node:fs';
import assert from 'node:assert/strict';
import path from 'node:path';
import {fileURLToPath,pathToFileURL} from 'node:url';
await import('../offline_guard.mjs');
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../..');
const source=path.join(root,'reference/ag1-executable/packages/coding-agent/dist/core');
const capability=await import(pathToFileURL(path.join(source,'model-capability-profile.js')));
const thinking=await import(pathToFileURL(path.join(source,'thinking-capability.js')));
const profiles=[];
for(const local of [false,true])for(const context of [null,8192,16383,16384,65535,65536,90000,127999,128000,199999,200000,262144])for(const max of [null,8192,32768,65536]) {
    const model={id:'name-does-not-set-capability',provider:local?'local':'remote',api:'openai-completions',baseUrl:local?'http://127.0.0.1:18080/v1':'https://fixture.invalid/v1',input:['text','image'],reasoning:true,contextWindow:context,maxTokens:max};
    const profile=capability.inferModelCapabilityProfile(model);
    profiles.push({local,context,max,profile,generation_max_tokens:capability.recommendGenerationMaxTokens(model),serialize_tools:capability.shouldSerializeToolCalls(profile),volume_rails:capability.shouldApplyVolumeRails(profile)});
}
const templates=[null,'plain text','{% if enable_thinking %}think{% endif %}',"{% if reasoning_effort not in ('none', 'low', 'medium', 'xhigh') %}",'enable_thinking is defined and enable_thinking is false',"enable_thinking\n{% if reasoning_effort in ['off', 'low', 'max'] %}",'not_enable_thinking_variable'];
const cases=[];
for(const reasoning of [false,true])for(const template of templates)for(const declared of [null,{modes:['OFF','HIGH'],default:'HIGH'},{modes:['off','custom'],apply:['reasoning_effort'],default:'custom'}]) {
    const model={provider:'llamacpp',baseUrl:'http://127.0.0.1:18080/v1',reasoning,thinking:declared};
    cases.push({reasoning,template,declared,resolved:thinking.resolveThinkingProfile(model,template??undefined)});
}
const result={version:1,source_commit:'e30f0b4b238ee5618928bcd17b4571eb046e7038',profiles,thinking:cases,network_actions:0};
const target=path.join(root,'fixtures/providers/capabilities.json');
if(process.argv.includes('--capture')){assert(!fs.existsSync(target));fs.writeFileSync(target,JSON.stringify(result,null,2)+'\n');}
else assert.deepEqual(result,JSON.parse(fs.readFileSync(target,'utf8')));
console.log(JSON.stringify({status:'passed',profiles:profiles.length,thinking:cases.length,network_actions:0}));
