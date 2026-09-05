import fs from 'node:fs';import assert from 'node:assert/strict';import path from 'node:path';import {fileURLToPath,pathToFileURL}from'node:url';
await import('../offline_guard.mjs');const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../..');
const mod=await import(pathToFileURL(path.join(root,'reference/ag1-executable/packages/coding-agent/dist/core/vision-capability.js')));
const cases=[
 ['llamacpp','probeLlamaCpp',{modalities:{vision:true}}],['llamacpp','probeLlamaCpp',{modalities:{vision:false}}],['llamacpp','probeLlamaCpp',{}],
 ['ollama','probeOllama',{capabilities:['completion','vision']}],['ollama','probeOllama',{capabilities:['completion']}],['ollama','probeOllama',{}],
 ['lmstudio','probeLmStudio',{data:[{id:'publisher/model',type:'vlm'}]}],['lmstudio','probeLmStudio',{data:[{id:'other/model',type:'llm'},{id:'model',type:'vlm'}]}],['lmstudio','probeLmStudio',{data:[{id:'other/wrong',type:'vlm'}]}],
 ['canary','probeCanary',{choices:[{message:{content:'teal'}}]}],['canary','probeCanary',{choices:[{message:{content:[{type:'text',text:'teal'}]}}]}],['canary','probeCanary',{choices:[{message:{content:'red'}}]}],['canary','probeCanary',{}],
];
const groups=[];
for(const [kind,symbol,payload]of cases)for(const status of [200,400,404]){
 let request;const bytes=JSON.stringify(payload);
 const result=await mod[symbol]({provider:kind,baseUrl:'https://fixture.invalid/v1',modelId:'model',apiKey:'fixture-key',fetchFn:async(input,options)=>{const r=new Request(input,options);const body=await r.text();request={url:r.url,method:r.method,body:body?JSON.parse(body):null};return new Response(bytes,{status});}});
 groups.push({kind,model:'model',status,bytes,result,request});
}
for(const bytes of ['model alias not found','model does not exist','bad image']){const result=await mod.probeCanary({provider:'custom',baseUrl:'https://fixture.invalid/v1',modelId:'model',fetchFn:async()=>new Response(bytes,{status:400})});groups.push({kind:'canary',model:'model',status:400,bytes,result});}
const out={version:1,source_commit:'e30f0b4b238ee5618928bcd17b4571eb046e7038',groups,network_actions:0};const target=path.join(root,'fixtures/providers/vision.json');if(process.argv.includes('--capture')){assert(!fs.existsSync(target));fs.writeFileSync(target,JSON.stringify(out,null,2)+'\n');}else assert.deepEqual(out,JSON.parse(fs.readFileSync(target,'utf8')));console.log(JSON.stringify({status:'passed',vision_cases:groups.length,network_actions:0}));
