import fs from 'node:fs';import path from 'node:path';import assert from 'node:assert/strict';import {fileURLToPath,pathToFileURL} from 'node:url';
await import('../offline_guard.mjs');const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../..');
const {createReadToolDefinition}=await import(pathToFileURL(path.join(root,'reference/ag1-executable/packages/coding-agent/dist/core/tools/read.js')));
const cwd=fs.mkdtempSync(path.join(root,'.local/p09-stream-read-'));const definition=createReadToolDefinition(cwd);
const inputs=[
 {prefix:'\ufeff',unit:'line 🦙\r\n',repeat:900000,suffix:'tail β',args:{}},
 {prefix:'\ufeff',unit:'line 🦙\r\n',repeat:900000,suffix:'tail β',args:{offset:800001,limit:3}},
 {prefix:'\ufeff',unit:'line 🦙\r\n',repeat:900000,suffix:'tail β',args:{offset:900001}},
 {unit:'x',repeat:9000000,args:{}},
 {prefix:'first\n',unit:'x',repeat:9000000,suffix:'\nlast',args:{}},
 {unitBase64:'/wo=',repeat:4300000,args:{limit:2}},
 {unit:'line\n',repeat:1800000,args:{offset:1800002}},
];
const rows=[];
try{for(const [i,input]of inputs.entries()){
 const unit=input.unitBase64?Buffer.from(input.unitBase64,'base64'):Buffer.from(input.unit);const buffer=Buffer.alloc(unit.length*input.repeat);for(let pos=0;pos<buffer.length;pos+=unit.length)unit.copy(buffer,pos);
 fs.writeFileSync(path.join(cwd,'large.txt'),Buffer.concat([Buffer.from(input.prefix??''),buffer,Buffer.from(input.suffix??'')]));
 const args={path:'large.txt',...input.args};let output;try{output={result:await definition.execute(`stream-${i}`,args)};}catch(e){output={error:e.message};}
 rows.push({input:{...input,args},output});
}}finally{fs.rmSync(cwd,{recursive:true,force:true});}
const result={version:1,source_commit:'e30f0b4b238ee5618928bcd17b4571eb046e7038',rows,model_actions:0,network_actions:0};const target=path.join(root,'fixtures/tools/stream-read-oracle.json');
if(process.argv.includes('--capture')){assert(!fs.existsSync(target));fs.writeFileSync(target,JSON.stringify(result,null,2)+'\n');}else assert.deepEqual(result,JSON.parse(fs.readFileSync(target,'utf8')));
console.log(JSON.stringify({status:'passed',stream_read_cases:rows.length,model_actions:0,network_actions:0}));
