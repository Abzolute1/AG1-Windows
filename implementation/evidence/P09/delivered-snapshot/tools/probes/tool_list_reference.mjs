import fs from 'node:fs';import path from 'node:path';import assert from 'node:assert/strict';import {fileURLToPath,pathToFileURL} from 'node:url';
await import('../offline_guard.mjs');const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../..');
const {createLsToolDefinition}=await import(pathToFileURL(path.join(root,'reference/ag1-executable/packages/coding-agent/dist/core/tools/ls.js')));
const cases=[{entries:[],args:{}},{entries:['a','B','b','A','.hidden','_under','-dash','1','10','2','é','è','e','E','🦙','β','文','dir/'],args:{}},{entries:['one','two','three/'],args:{limit:1}},{entries:['one','two'],args:{limit:2}},{entries:['inner/a','inner/b/'],args:{path:'inner'}},{entries:Array.from({length:505},(_,i)=>`entry-${i}`),args:{}},{entries:Array.from({length:400},(_,i)=>`${'x'.repeat(200)}-${i}`),args:{limit:1000}}];
const cwd=fs.mkdtempSync(path.join(root,'.local/p09-lists-'));const rows=[];
try{for(const [i,input]of cases.entries()){
 const dir=path.join(cwd,String(i));fs.mkdirSync(dir);
 for(const entry of input.entries){const target=path.join(dir,entry);if(entry.endsWith('/'))fs.mkdirSync(target,{recursive:true});else{fs.mkdirSync(path.dirname(target),{recursive:true});fs.writeFileSync(target,'fixture');}}
 const output=await createLsToolDefinition(dir).execute('list',input.args);
 rows.push({input,output:JSON.parse(JSON.stringify(output))});
}}finally{fs.rmSync(cwd,{recursive:true,force:true});}
const out={version:1,source_commit:'e30f0b4b238ee5618928bcd17b4571eb046e7038',locale:new Intl.Collator().resolvedOptions().locale,model_actions:0,network_actions:0,rows};const target=path.join(root,'fixtures/tools/list-oracle.json');
if(process.argv.includes('--capture')){assert(!fs.existsSync(target));fs.writeFileSync(target,JSON.stringify(out,null,2)+'\n');}else assert.deepEqual(out,JSON.parse(fs.readFileSync(target,'utf8')));
console.log(JSON.stringify({status:'passed',list_cases:rows.length,model_actions:0,network_actions:0}));
