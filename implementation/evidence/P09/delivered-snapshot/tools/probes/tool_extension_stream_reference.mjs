import fs from 'node:fs';import path from 'node:path';import assert from 'node:assert/strict';import {fileURLToPath,pathToFileURL} from 'node:url';
await import('../offline_guard.mjs');const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../..');
const {AssistantMessageEventStream}=await import(pathToFileURL(path.join(root,'reference/ag1-executable/packages/ai/dist/utils/event-stream.js')));
const message={role:'assistant',content:[{type:'text',text:'fixture'}],api:'fixture',provider:'fixture',model:'fixture',usage:{input:0,output:0,cacheRead:0,cacheWrite:0,totalTokens:0,cost:{input:0,output:0,cacheRead:0,cacheWrite:0,total:0}},stopReason:'stop',timestamp:12};
const cases=[
 {id:'queued-done',events:[{type:'start',partial:message},{type:'done',reason:'stop',message}]},
 {id:'queued-error',events:[{type:'error',reason:'error',error:{...message,stopReason:'error',errorMessage:'fixture'}}]},
 {id:'terminal-first-wins',events:[{type:'done',reason:'stop',message},{type:'done',reason:'stop',message:{...message,model:'late'}}]},
 {id:'explicit-end',events:[{type:'start',partial:message}],end:message},
 {id:'fallback-end',events:[]},
 {id:'waiting-consumer',events:[{type:'start',partial:message},{type:'done',reason:'stop',message}],waiting:true},
];
const rows=[];const prior=Date.now;const env=[process.env.AG1_STREAM_STALL_TIMEOUT_MS,process.env.AG1_REASONING_STALL_TIMEOUT_MS];Date.now=()=>12;process.env.AG1_STREAM_STALL_TIMEOUT_MS='0';process.env.AG1_REASONING_STALL_TIMEOUT_MS='0';
try{for(const spec of cases){const stream=new AssistantMessageEventStream();let waiting;if(spec.waiting){const iterator=stream[Symbol.asyncIterator]();waiting=iterator.next();for(const event of spec.events)stream.push(event);stream.end(spec.end);const first=await waiting;const events=[first.value];for await(const event of iterator)events.push(event);rows.push({...spec,output:{events,result:await stream.result()}});}else{for(const event of spec.events)stream.push(event);stream.end(spec.end);const events=[];for await(const event of stream)events.push(event);rows.push({...spec,output:{events,result:await stream.result()}});}}}finally{Date.now=prior;for(const [i,key]of ['AG1_STREAM_STALL_TIMEOUT_MS','AG1_REASONING_STALL_TIMEOUT_MS'].entries()){if(env[i]===undefined)delete process.env[key];else process.env[key]=env[i];}}
const result={version:1,source_commit:'e30f0b4b238ee5618928bcd17b4571eb046e7038',rows};const target=path.join(root,'fixtures/tools/extension-stream-oracle.json');if(process.argv.includes('--capture')){assert(!fs.existsSync(target));fs.writeFileSync(target,JSON.stringify(result,null,2)+'\n');}else assert.deepEqual(result,JSON.parse(fs.readFileSync(target,'utf8')));console.log(JSON.stringify({status:'passed',assistant_stream_cases:rows.length,network_actions:0,model_actions:0}));
