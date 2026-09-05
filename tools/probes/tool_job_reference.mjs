import fs from 'node:fs';import path from 'node:path';import assert from 'node:assert/strict';import {fileURLToPath,pathToFileURL} from 'node:url';
await import('../offline_guard.mjs');const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../..');const cwd=fs.mkdtempSync(path.join(root,'.local/p09-jobs-'));
const base=path.join(root,'reference/ag1-executable/packages/coding-agent/dist/core');
const {createJobToolDefinition}=await import(pathToFileURL(path.join(base,'tools/job.js')));const registry=await import(pathToFileURL(path.join(base,'job-registry.js')));
const realNow=Date.now;Date.now=()=>10000;const definition=createJobToolDefinition(cwd);const rows=[];const ids=[];
async function run(input){let output;try{output={result:await definition.execute('job-fixture',input)};}catch(e){output={error:e.message};}rows.push({input,output});return output;}
try{
 await run({action:'list'});await run({action:'start',command:''});await run({action:'poll'});await run({action:'poll',id:'missing'});
 for(const command of ['printf job-output','printf failed; exit 3']){
  const start=await run({action:'start',command});const id=start.result.details.id;ids.push(id);
  for(let i=0;i<500;i++){const job=registry.pollJob(cwd,id);if(job.status==='exited')break;await new Promise(r=>setTimeout(r,10));}
  assert.equal(registry.pollJob(cwd,id).status,'exited');await run({action:'poll',id});await run({action:'log',id});await run({action:'kill',id});
 }
 await run({action:'list'});
 const notice=registry.formatFinishedJobsAddendum(cwd);const duplicate=registry.formatFinishedJobsAddendum(cwd)??null;
 const formats=['running','exited','unknown'].flatMap(status=>['short','x'.repeat(90),'β🦙'.repeat(50)].map(command=>({status,command,output:registry.formatJobLine({id:'fixture',status,command,startedAt:8000,exitCode:3})})));
 let value={version:1,source_commit:'e30f0b4b238ee5618928bcd17b4571eb046e7038',rows,notice,duplicate,formats,model_actions:0,network_actions:0};
 let encoded=JSON.stringify(value);for(const [i,id]of ids.entries())encoded=encoded.replaceAll(id,`job-${i}`);encoded=encoded.replace(/\(pid \d+\)/g,'(pid 42)');value=JSON.parse(encoded);
 const wellFormed=v=>typeof v==='string'?v.toWellFormed():Array.isArray(v)?v.map(wellFormed):v&&typeof v==='object'?Object.fromEntries(Object.entries(v).map(([k,x])=>[k,wellFormed(x)])):v;value=wellFormed(value);value.unicode_transport='Unpaired UTF-16 surrogates become U+FFFD at the Rust UTF-8 boundary.';
 const target=path.join(root,'fixtures/tools/job-oracle.json');if(process.argv.includes('--capture')){assert(!fs.existsSync(target));fs.writeFileSync(target,JSON.stringify(value,null,2)+'\n');}else assert.deepEqual(value,JSON.parse(fs.readFileSync(target,'utf8')));
 console.log(JSON.stringify({status:'passed',job_cases:rows.length,format_cases:formats.length,model_actions:0,network_actions:0}));
}finally{Date.now=realNow;fs.rmSync(cwd,{recursive:true,force:true});}
