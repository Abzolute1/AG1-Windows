import fs from 'node:fs';import os from 'node:os';import path from 'node:path';import assert from 'node:assert/strict';import {fileURLToPath,pathToFileURL}from'node:url';
await import('../offline_guard.mjs');
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../..');
const runtime=await import(pathToFileURL(path.join(root,'reference/ag1-executable/packages/coding-agent/dist/core/llamacpp-runtime.js')));
const names=['model.gguf','model.GGUF','mmproj-model.gguf','MTP-model.gguf','downloading_model.gguf','model.gguf.part','model-00001-of-00003.gguf','model-00003-of-00003.GGUF','model-00000-of-00003.gguf','model-00001-of-00001.gguf','-00001-of-00003.gguf','Qwen3.6-35B-A3B-UD-Q4_K_XL.gguf','mmproj-Qwen3.6-35B-A3B-bf16.gguf','downloading_mmproj-Qwen3.8-IQ4_XS.gguf','X-Q4_0.gguf','X-q4_k_m-f16.gguf'];
const pure=names.map(name=>({name,usable:runtime.isUsableGgufName(name),split:runtime.parseSplitGgufName(name)??null,family:runtime.llamaCppFamilyKey(name)}));
const tuning=[];for(const currentN of [0,20,42,60])for(const headroomMiB of [-33000,-825,-275,0,275,825,33000])tuning.push({currentN,headroomMiB,expected:runtime.computeTunedNCpuMoe({currentN,headroomMiB})});
const dir=fs.mkdtempSync(path.join(os.tmpdir(),'ag1-discovery-'));let discovery;
const files={'Qwen3.6-35B-A3B-UD-Q4_K_XL.gguf':12,'mmproj-Qwen3.6-35B-A3B-f16.gguf':4,'other/mmproj-Qwen3.6-35B-A3B-bf16.gguf':8,'Flash-00001-of-00003.gguf':3,'Flash-00002-of-00003.gguf':4,'Flash-00003-of-00003.gguf':5,'Broken-00001-of-00002.gguf':3,'Zero-00001-of-00002.gguf':4,'Zero-00002-of-00002.gguf':0,'mtp-head.gguf':5,'downloading_model.gguf':2,'.hidden/hidden.gguf':9};
try{
 for(const [name,size]of Object.entries(files)){const p=path.join(dir,name);fs.mkdirSync(path.dirname(p),{recursive:true});fs.writeFileSync(p,Buffer.alloc(size));}
 const models=runtime.discoverLlamaCppModels([dir]).map(m=>({id:m.id,path:path.relative(dir,m.path),sizeBytes:m.sizeBytes}));
 const selected=runtime.discoverLlamaCppMmproj({modelPath:path.join(dir,'Qwen3.6-35B-A3B-UD-Q4_K_XL.gguf'),searchDirs:[dir],env:{}});
 discovery={files,models,projector:path.relative(dir,selected)};
}finally{fs.rmSync(dir,{recursive:true,force:true});}
const out={version:1,source_commit:'e30f0b4b238ee5618928bcd17b4571eb046e7038',pure,tuning,discovery,model_actions:0};const target=path.join(root,'fixtures/providers/discovery.json');
if(process.argv.includes('--capture')){assert(!fs.existsSync(target));fs.writeFileSync(target,JSON.stringify(out,null,2)+'\n');}else assert.deepEqual(out,JSON.parse(fs.readFileSync(target,'utf8')));
console.log(JSON.stringify({status:'passed',name_cases:pure.length,tuning_cases:tuning.length,model_actions:0}));
