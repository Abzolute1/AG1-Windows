import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {fileURLToPath,pathToFileURL} from 'node:url';
await import('../offline_guard.mjs');
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../..');
const base=path.join(root,'reference/ag1-executable/packages/coding-agent');
const {ToolOrchestrator}=await import(pathToFileURL(path.join(base,'dist/core/tool-orchestrator.js')));
const policy={mode:'strict_benchmark',intent:'debug_fix',mutationExpected:true,webRelevant:false,allowedPaths:['src/value.js','src/support.js','tests/value.test.js'],expectedChangedPaths:['src/value.js'],likelyTests:['tests/value.test.js'],verification:'node --test',maxReadsBeforePatch:4,maxSearchesBeforePatch:1,maxPatchAttempts:2,stopAfterVerifierPass:true};
const check=(name,args={})=>({kind:'evaluate',name,args});
const note=(name,args={},isError=false,output='ok')=>({kind:'observe',name,args,isError,output});
const read=p=>note('read',{path:p});
const write=p=>note('write',{path:p});
const verify=(error=false)=>note('bash',{command:'node --test'},error,error?'AssertionError: expected 1 got 2':'# tests 1\n# pass 1\n# fail 0');
const cases=[
 {id:'strict-evidence',actions:[check('ls'),check('find',{pattern:'**/*'}),check('read',{path:'tests/value.test.js'}),read('tests/value.test.js'),read('src/value.js'),check('read',{path:'src/support.js'}),check('write',{path:'src/value.js'}),write('src/value.js'),check('write',{path:'src/value.js'}),check('bash',{command:'node --test'}),verify(),check('read',{path:'src/value.js'})]},
 {id:'search-budget',actions:[check('grep'),note('grep',{},true),check('grep'),note('grep'),check('grep'),check('find',{pattern:'*.js'}),write('src/value.js'),check('grep')]},
 {id:'read-budget',policy:{expectedChangedPaths:[],likelyTests:[],allowedPaths:['a.js'],maxReadsBeforePatch:2},actions:[read('one.js'),read('two.js'),check('read',{path:'third.js'}),check('read',{path:'a.js'}),read('a.js'),check('read',{path:'a.js'})]},
 {id:'mutation-set',actions:[check('write',{path:'src/other.js'}),check('bash',{command:'node --test'}),write('src/value.js'),check('edit',{path:'src/value.js'}),verify(true),check('edit',{path:'src/value.js'})]},
 {id:'repair',actions:[write('src/value.js'),verify(true),check('bash',{command:'node --test'}),write('src/value.js'),verify(true),check('write',{path:'src/value.js'}),check('bash',{command:'node --test'})]},
 {id:'discovery',actions:[check('bash',{command:'pwd'}),check('bash',{command:'git status --short'}),check('bash',{command:'echo hello; ls'}),check('bash',{command:'printf hello'}),check('bash')]},
 {id:'controlled',policy:{mode:'controlled'},actions:[check('ls'),check('find',{pattern:'*'}),read('a.js'),read('b.js'),read('c.js'),check('read',{path:'d.js'}),check('write',{path:'other.js'}),write('other.js'),check('write',{path:'other.js'})]},
 {id:'readonly',policy:{mode:'controlled',intent:'code_explanation',mutationExpected:false,expectedChangedPaths:[],verification:undefined,stopAfterVerifierPass:false},actions:[check('write',{path:'src/value.js'}),check('read',{path:'src/value.js'}),read('src/value.js'),check('ls')]},
 {id:'flexible',policy:{mode:'flexible',mutationExpected:false},actions:[check('write',{path:'other.js'}),check('ls'),check('bash'),write('src/value.js'),verify(),check('read',{path:'src/value.js'})]},
 {id:'verifier-before-mutation',actions:[verify(),check('read',{path:'src/value.js'}),write('src/value.js'),verify(),check('read',{path:'src/value.js'})]},
 {id:'stop-disabled',policy:{stopAfterVerifierPass:false},actions:[write('src/value.js'),verify(),check('read',{path:'src/support.js'})]},
];
const cwd=fs.mkdtempSync(path.join(os.tmpdir(),'ag1-orchestrator-'));
const rows=[];
try{for(const item of cases){const selected={...policy,...item.policy};const controller=new ToolOrchestrator({cwd,policy:selected});const actions=[];for(const action of item.actions){let decision=null;const args=structuredClone(action.args);if(action.kind==='evaluate')decision=controller.evaluate(action.name,args);else controller.noteToolResult({toolName:action.name,args,isError:action.isError,result:{content:[{type:'text',text:action.output}]}});actions.push({input:action,decision,args,hint:controller.getNextActionHint()??null,stop:controller.getStopDecision()});}rows.push({id:item.id,policy:selected,actions});}}finally{fs.rmSync(cwd,{recursive:true,force:true});}
const source_files=['tool-orchestrator.ts','task-policy.ts','evidence-planner.ts','repair-controller.ts','stop-controller.ts','verification-controller.ts'].map(name=>({path:'packages/coding-agent/src/core/'+name,sha256:createHash('sha256').update(fs.readFileSync(path.join(base,'src/core',name))).digest('hex')}));
const result=JSON.parse(JSON.stringify({version:1,source_commit:'e30f0b4b238ee5618928bcd17b4571eb046e7038',source_files,rows,model_actions:0,network_actions:0}));
const target=path.join(root,'fixtures/runtime/orchestrator-oracle.json');
if(process.argv.includes('--capture')){assert(!fs.existsSync(target));fs.writeFileSync(target,JSON.stringify(result,null,2)+'\n');}else assert.deepEqual(result,JSON.parse(fs.readFileSync(target,'utf8')));
console.log(JSON.stringify({status:'passed',traces:rows.length,actions:rows.reduce((n,r)=>n+r.actions.length,0),model_actions:0,network_actions:0}));
