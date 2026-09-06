// Execute the unchanged pinned repository index and task-policy selection.
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {fileURLToPath,pathToFileURL} from 'node:url';
await import('../offline_guard.mjs');
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../..');
const base=path.join(root,'reference/ag1-executable/packages/coding-agent');
const {buildWorkspaceIndex}=await import(pathToFileURL(path.join(base,'dist/core/workspace-index.js')));
const {inferTaskPolicy}=await import(pathToFileURL(path.join(base,'dist/core/task-policy.js')));
const bt=String.fromCharCode(96);
const cases=[
 {id:'empty',files:{}},
 {id:'npm-test',files:{'package.json':JSON.stringify({scripts:{test:'node --test',check:'tsc',invalid:42}}),'Cargo.toml':''}},
 {id:'npm-check',files:{'package.json':JSON.stringify({scripts:{test:'',check:'tsc'}})}},
 {id:'bad-package',files:{'package.json':'{broken','Cargo.toml':''}},
 {id:'array-scripts',files:{'package.json':'{"scripts":["test"]}','go.mod':''}},
 ...['.defects4j.config','mvnw','pom.xml','gradlew','build.gradle','build.gradle.kts','Cargo.toml','go.mod','pyproject.toml','requirements.txt'].map(marker=>({id:'marker-'+marker,files:{[marker]:''}})),
 {id:'defects-priority',files:{'nested/.defects4j.config':'','package.json':'{"scripts":{"test":"node --test"}}'}},
 {id:'nested-quote',files:{"my project's/Cargo.toml":'','other/deeper/Cargo.toml':''}},
 {id:'marker-order',files:{'Cargo.toml':'','deep/mvnw':''}},
 {id:'root-priority',files:{'Cargo.toml':''},bulk:1005},
 {id:'one-node-test',files:{'src/value.js':'export const answer=42;','test/value.test.js':"import {answer} from '../src/value.js';"}},
 {id:'many-node-tests',files:{'a.test.js':'','b.test.js':''}},
 {id:'only-typescript',files:{'a.test.ts':''}},
 {id:'ignored',files:{'.gitignore':'ignored/\n*.skip.js\n!a.skip.js\n','ignored/Cargo.toml':'','a.skip.js':'','node_modules/a.test.js':'','test/a.test.js':''}},
 {id:'imports',prompt:'Fix the deep value.',files:{'src/a.ts':"export {b} from './b';",'src/b/index.ts':'export const b=1;','src/deep.js':"const b = require('./b'); import('./a');",'tests/deep.test.cjs':"require('../src/deep.js');"}},
 {id:'unicode-order',files:{'é.test.js':'','Z.test.js':'','a.test.js':'','Å.test.js':''}},
 {id:'explicit',prompt:'Fix it. Intended verification command is: '+bt+'cargo test -p selected'+bt,files:{'package.json':'{"scripts":{"test":"npm test"}}'}},
 {id:'targeted',prompt:'Fix it. Run the targeted verification command: pytest tests/unit.',files:{}},
 {id:'run-before',prompt:'Fix it and run '+bt+'node --test exact.test.js'+bt+' before finishing.',files:{'go.mod':''}},
 {id:'manifest-cap',prompt:'Fix target behavior.',files:{'src/target.js':'export const target=1;'},testBulk:26},
];
const rows=[];
for(const item of cases){
 const input={prompt:'Fix the implementation.',...item};
 const cwd=fs.mkdtempSync(path.join(os.tmpdir(),'ag1-verifier-oracle-'));
 try{
  const files={...input.files};
  for(let i=0;i<(input.bulk??0);i++)files['aaa/file-'+String(i).padStart(4,'0')+'.txt']='';
  for(let i=0;i<(input.testBulk??0);i++)files['test/'+String(i).padStart(2,'0')+'.test.js']="import '../src/target.js';";
  for(const [name,bytes]of Object.entries(files)){const target=path.join(cwd,name);fs.mkdirSync(path.dirname(target),{recursive:true});fs.writeFileSync(target,bytes);}
  const index=buildWorkspaceIndex({rootDir:cwd,useCache:false});
  const verifier=inferTaskPolicy({prompt:input.prompt,index}).verification??null;
  rows.push({input,index:{files:index.files.map(file=>file.path),testFiles:index.testFiles,packageScripts:index.packageScripts,importEdges:index.importEdges},verifier});
 }finally{fs.rmSync(cwd,{recursive:true,force:true});}
}
const sources=['workspace-index.ts','task-policy.ts','verification-controller.ts'];
const result={version:1,source_commit:'e30f0b4b238ee5618928bcd17b4571eb046e7038',source_files:sources.map(name=>({path:'packages/coding-agent/src/core/'+name,sha256:createHash('sha256').update(fs.readFileSync(path.join(base,'src/core',name))).digest('hex')})),rows,model_actions:0,network_actions:0};
const target=path.join(root,'fixtures/runtime/verifier-oracle.json');
if(process.argv.includes('--capture')){assert(!fs.existsSync(target));fs.writeFileSync(target,JSON.stringify(result,null,2)+'\n');}
else assert.deepEqual(result,JSON.parse(fs.readFileSync(target,'utf8')));
console.log(JSON.stringify({status:'passed',cases:rows.length,model_actions:0,network_actions:0}));
