// Actual unchanged SDK tool, immutable artifact store, and in-memory session port.
import fs from 'node:fs';import path from 'node:path';import os from 'node:os';import assert from 'node:assert/strict';import {createHash} from 'node:crypto';import {fileURLToPath,pathToFileURL} from 'node:url';
await import('../offline_guard.mjs');
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../..');
const base=path.join(root,'reference/ag1-executable/packages/coding-agent');
const {ArtifactStore}=await import(pathToFileURL(path.join(base,'dist/core/artifact-store.js')));
const {createSourceContextToolDefinition}=await import(pathToFileURL(path.join(base,'dist/core/tools/source-context.js')));
const temporary=fs.mkdtempSync(path.join(os.tmpdir(),'ag1-source-tool-reference-'));
const rows=[];const originalDate=globalThis.Date;const now='2026-09-06T00:00:00.000Z';
try {
 globalThis.Date=class extends originalDate {constructor(...args){super(...(args.length?args:[now]));}static now(){return new originalDate(now).getTime();}};
 const workspace=path.join(temporary,'workspace');fs.mkdirSync(workspace);
 const fixture=JSON.parse(fs.readFileSync(path.join(root,'fixtures/context/source-inputs.json'),'utf8'));
 for(const [name,text] of Object.entries(fixture.files)){const target=path.join(workspace,name);fs.mkdirSync(path.dirname(target),{recursive:true});fs.writeFileSync(target,text);}
 const branch=[];const artifacts=new ArtifactStore(path.join(temporary,'artifacts'),'fixture');
 const tool=createSourceContextToolDefinition(workspace,{artifactStore:artifacts,sessionManager:{getBranch:()=>branch,appendCustomEntryAsync:async(customType,data)=>{branch.push({type:'custom',customType,data});return 'owned-capsule-entry';}}});
 async function capture(id,args){const before=branch.length;let output,error;try{output=await tool.execute('source-call',args);}catch(caught){error=true;}rows.push({id,args,...(error?{error}:{output}),writes:branch.length-before});return output;}
 await capture('empty-list',{action:'list'});
 await capture('unknown-status',{action:'status',capsuleId:'cap_0000000000000000'});
 const build=await capture('build',fixture.cases.find(c=>c.id==='explicit-current').input);
 const id=build.details.capsuleId;
 await capture('current-status',{action:'status',capsuleId:id});
 await capture('current-list',{action:'list'});
 fs.writeFileSync(path.join(workspace,'src/compiler.ts'),'export class ChangedCompiler {}\n');
 await capture('stale-status',{action:'status',capsuleId:id});
 await capture('stale-list',{action:'list'});
 fs.rmSync(path.join(workspace,'src/compiler.ts'));
 await capture('missing-status',{action:'status',capsuleId:id});
 const normalized=JSON.parse(JSON.stringify(rows).replaceAll(build.details.artifactId,'art_00000000000000000000000000000001'));
 const result={version:1,source_commit:'e30f0b4b238ee5618928bcd17b4571eb046e7038',source_files:['tools/source-context','context-capsule','source-retrieval-signals','source-revision'].map(name=>({path:`packages/coding-agent/src/core/${name}.ts`,sha256:createHash('sha256').update(fs.readFileSync(path.join(base,`src/core/${name}.ts`))).digest('hex')})),rows:normalized,model_actions:0,network_actions:0};
 const target=path.join(root,'fixtures/runtime/source-context-tool-oracle.json');
 if(process.argv.includes('--capture')){assert(!fs.existsSync(target));fs.writeFileSync(target,JSON.stringify(result,null,2)+'\n');}else assert.deepEqual(result,JSON.parse(fs.readFileSync(target,'utf8')));
 console.log(JSON.stringify({status:'passed',source_context_cases:rows.length,model_actions:0,network_actions:0}));
}finally{globalThis.Date=originalDate;fs.rmSync(temporary,{recursive:true,force:true});}
