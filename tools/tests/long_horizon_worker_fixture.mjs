// Exercise the real worker/TS SDK through scripted HTTP and owner-control peers.
// This fixture cannot create a live or duration acceptance report.
import fs from 'node:fs';import path from 'node:path';import net from 'node:net';import http from 'node:http';import assert from 'node:assert/strict';import crypto from 'node:crypto';import {spawn} from 'node:child_process';import {pathToFileURL} from 'node:url';
const root=process.cwd(),out=path.resolve(process.argv[2]);assert.equal(path.dirname(out),path.join(root,'.local/p14-long-horizon'));fs.mkdirSync(out,{mode:0o700});
const prepared=path.join(root,'.local/p14-long-horizon-preparation-3/fixture-generator.mjs');
const {createBenchmark}=await import(pathToFileURL(prepared));const {CHECKPOINT_SPECS:specs}=createBenchmark({});
let calls=0,preparations=0,tokenizations=0,httpSerial=0;const secret=crypto.randomUUID(),socketPath=path.join(out,'control.sock');
const control=net.createServer(socket=>{let text='';socket.on('data',chunk=>{text+=chunk;if(text.includes('\n')){const v=JSON.parse(text);assert.deepEqual(v,{key:secret,action:'prepare',model:'p14-scripted-owner'});preparations++;socket.end(JSON.stringify({prepared:true})+'\n');}});});
await new Promise(resolve=>control.listen(socketPath,resolve));
const server=http.createServer(async(req,res)=>{
 let raw='';for await(const chunk of req)raw+=chunk;
 if(req.url==='/tokenize') {const body=JSON.parse(raw);tokenizations++;res.setHeader('content-type','application/json');res.end(JSON.stringify({tokens:Array(Math.ceil(body.content.length/4)).fill(1)}));return;}
 if(req.url==='/props'){res.setHeader('content-type','application/json');res.end('{}');return;}
 assert.equal(req.url,'/v1/chat/completions');fs.writeFileSync(path.join(out,`http-${++httpSerial}.json`),raw);assert(calls<30);assert(preparations>calls,'Provider request needs an owner preparation');
 const i=calls++,spec=specs[Math.floor(i/5)],step=i%5;
 const fixed=spec.id==='recovery'?'export function recoveryKey(service, attempt) { return String(service).toLowerCase() + "|" + attempt; }\n':spec.initialSource;
 const actions=[['artifact',{action:'search',query:spec.marker}],['read',{path:spec.sourcePath}],['edit',{path:spec.sourcePath,oldText:spec.brokenSource,newText:fixed}],['bash',{command:`node ${spec.verification}`}]];
 const delta=step<4?{tool_calls:[{index:0,id:`fixture-${i}`,type:'function',function:{name:actions[step][0],arguments:JSON.stringify(actions[step][1])}}]}:{content:'Scripted fixture complete.'};
 res.setHeader('content-type','text/event-stream');res.end(`data: ${JSON.stringify({choices:[{index:0,delta,finish_reason:step<4?'tool_calls':'stop'}],usage:{prompt_tokens:100,completion_tokens:10,total_tokens:110}})}\n\ndata: [DONE]\n\n`);
});await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
const model={id:'p14-scripted-owner',name:'Scripted owner fixture',provider:'llamacpp',api:'openai-completions',baseUrl:`http://127.0.0.1:${server.address().port}/v1`,reasoning:false,input:['text'],contextWindow:32768,maxTokens:2048,cost:{input:0,output:0,cacheRead:0,cacheWrite:0}};
const input={fixture:true,model,source:path.join(root,'reference/ag1-executable'),destination:out,control:{socket:socketPath,key:secret},workload:{mode:'seed-and-typescript',rawTokens:2500000,phases:64,generated:{path:prepared,sha256:crypto.createHash('sha256').update(fs.readFileSync(prepared)).digest('hex')}}};
const file=path.join(out,'input.json');fs.writeFileSync(file,JSON.stringify(input),{flag:'wx'});const log=fs.openSync(path.join(out,'worker.log'),'wx');
let code;
try {code=await new Promise((resolve,reject)=>{const child=spawn(process.execPath,['tools/release_long_horizon_worker.mjs',file],{cwd:root,env:{...process.env,PATH:path.dirname(process.execPath)+':/usr/bin:/bin'},stdio:['ignore',log,log]});const timer=setTimeout(()=>{child.kill('SIGTERM');},180000);child.on('error',reject);child.on('exit',code=>{clearTimeout(timer);resolve(code);});});}
finally {fs.closeSync(log);server.closeAllConnections();await new Promise(resolve=>server.close(resolve));await new Promise(resolve=>control.close(resolve));}
assert.equal(code,0,fs.readFileSync(path.join(out,'worker.log'),'utf8'));
const result=JSON.parse(fs.readFileSync(path.join(out,'typescript-result.json')));assert.equal(result.fixture,true);assert.equal(result.status,'passed');assert.equal(result.release_acceptance,false);assert.equal(calls,30);assert(preparations>=37);assert(tokenizations>=64);
fs.writeFileSync(path.join(out,'fixture-result.json'),JSON.stringify({status:'passed-offline-preparation',fixture:true,release_acceptance:false,live_model_actions:0,scripted_calls:calls,owner_preparations:preparations,simulated_tokenizer_requests:tokenizations},null,2),{flag:'wx'});console.log('Supervised-worker protocol fixture passed six TS checkpoints; no live model actions.');
