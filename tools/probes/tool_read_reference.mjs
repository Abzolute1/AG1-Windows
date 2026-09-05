import fs from 'node:fs';import path from 'node:path';import assert from 'node:assert/strict';import {fileURLToPath,pathToFileURL} from 'node:url';
await import('../offline_guard.mjs');
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../..');
const read=await import(pathToFileURL(path.join(root,'reference/ag1-executable/packages/coding-agent/dist/core/tools/read.js')));
const inputs=JSON.parse(fs.readFileSync(path.join(root,'fixtures/tools/read-inputs.json'),'utf8'));
const cwd=fs.mkdtempSync(path.join(root,'.local/p09-read-'));const rows=[];
try{
 const definition=read.createReadToolDefinition(cwd);
 for(const [i,input] of inputs.entries()){
  fs.writeFileSync(path.join(cwd,'file.txt'),input.text);
  let output;try{output={result:await definition.execute('read-'+i,input.args)};}catch(e){output={error:e.message};}
  rows.push({input,output:JSON.parse(JSON.stringify(output))});
 }
}finally{fs.rmSync(cwd,{recursive:true,force:true});}
const out={version:1,source_commit:'e30f0b4b238ee5618928bcd17b4571eb046e7038',model_actions:0,network_actions:0,rows};const target=path.join(root,'fixtures/tools/read-oracle.json');
if(process.argv.includes('--capture')){assert(!fs.existsSync(target));fs.writeFileSync(target,JSON.stringify(out,null,2)+'\n');}else assert.deepEqual(out,JSON.parse(fs.readFileSync(target,'utf8')));
console.log(JSON.stringify({status:'passed',read_cases:rows.length,model_actions:0,network_actions:0}));
