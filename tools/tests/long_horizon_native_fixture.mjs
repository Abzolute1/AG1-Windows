// The official native preflight driver against a scripted server, never a model.
import fs from 'node:fs';import path from 'node:path';import http from 'node:http';import assert from 'node:assert/strict';import {spawn,spawnSync} from 'node:child_process';import {pathToFileURL} from 'node:url';
const root=process.cwd(),out=path.resolve(process.argv[2]),ts=path.join(root,'.local/p14-long-horizon/owner-fixture-3');
assert.equal(path.dirname(out),path.join(root,'.local/p14-long-horizon'));assert(!fs.existsSync(out));
const {createBenchmark}=await import(pathToFileURL(path.join(root,'.local/p14-long-horizon-preparation-3/fixture-generator.mjs')));const {CHECKPOINT_SPECS:specs}=createBenchmark({});let calls=0;
const server=http.createServer(async(req,res)=>{
 let raw='';for await(const chunk of req)raw+=chunk;
 assert.equal(req.url,'/v1/chat/completions');assert(calls<30);
 const i=calls++,spec=specs[Math.floor(i/5)],step=i%5;
 const fixed=spec.id==='recovery'?'export function recoveryKey(service, attempt) { return String(service).toLowerCase() + "|" + attempt; }\n':spec.initialSource;
 const actions=[['artifact',{action:'search',query:spec.marker}],['read',{path:spec.sourcePath}],['edit',{path:spec.sourcePath,oldText:spec.brokenSource,newText:fixed}],['bash',{command:`node ${spec.verification}`}]];
 const delta=step<4?{tool_calls:[{index:0,id:`fixture-${i}`,type:'function',function:{name:actions[step][0],arguments:JSON.stringify(actions[step][1])}}]}:{content:'Scripted fixture complete.'};
 res.setHeader('content-type','text/event-stream');res.end(`data: ${JSON.stringify({choices:[{index:0,delta,finish_reason:step<4?'tool_calls':'stop'}],usage:{prompt_tokens:100,completion_tokens:10,total_tokens:110}})}\n\ndata: [DONE]\n\n`);
});await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
const origin=`http://127.0.0.1:${server.address().port}/v1`;
const preparation=spawnSync('python3',['-c',`import sys,json\nfrom pathlib import Path\nsys.path.insert(0,'tools')\nfrom release_long_horizon import native_input\nroot=Path.cwd();ts=Path(sys.argv[1]);destination=Path(sys.argv[2]);v=json.load(open(ts/'input.json'));model=v['model'];model['baseUrl']=sys.argv[3];live=json.load(open(root/'.local/p14-quality/diagnosis-live-2/input.json'))['live'];live['models']=[model];native_input(ts,destination,live,model,fixture=True)`,ts,out,origin],{cwd:root,encoding:'utf8'});assert.equal(preparation.status,0,preparation.stderr);
const log=fs.openSync(path.join(out,'native.log'),'wx');let code;
try {code=await new Promise((resolve,reject)=>{const child=spawn(path.join(root,'.local/p14-quality-target/release/examples/release-long-horizon-native'),[path.join(out,'input.json')],{cwd:root,stdio:['ignore',log,log]});const timer=setTimeout(()=>child.kill('SIGTERM'),180000);child.on('error',reject);child.on('exit',code=>{clearTimeout(timer);resolve(code);});});}
finally{fs.closeSync(log);server.closeAllConnections();await new Promise(resolve=>server.close(resolve));}
assert.equal(code,0,fs.readFileSync(path.join(out,'native.log'),'utf8'));const result=JSON.parse(fs.readFileSync(path.join(out,'native-result.json')));assert.equal(result.fixture,true);assert.equal(result.status,'passed');assert.equal(result.request_count,30);assert.equal(calls,30);assert.equal(result.release_acceptance,false);assert.equal(result.checkpoints.length,6);assert(result.all_original_artifact_hashes_intact);console.log('Official native preflight driver passed six scripted checkpoints and captured all 30 outgoing requests.');
