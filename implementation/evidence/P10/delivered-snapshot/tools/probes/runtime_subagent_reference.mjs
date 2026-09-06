// Pure profile/limit references and owned checkpoint fault probes; no model calls.
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {fileURLToPath,pathToFileURL} from 'node:url';
import {execFileSync} from 'node:child_process';
await import('../offline_guard.mjs');
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../..');
const base=path.join(root,'reference/ag1-executable/packages/coding-agent');
const sub=await import(pathToFileURL(path.join(base,'dist/core/subagent-runner.js')));
const checkpoints=await import(pathToFileURL(path.join(base,'dist/core/checkpoint-manager.js')));
const cwd=fs.mkdtempSync(path.join(os.tmpdir(),'ag1-subagent-reference-'));
const profiles=[];const checkpointRows=[];let defects;
try {
 for(const type of sub.SUBAGENT_TYPES) {
  profiles.push({type,tools:sub.buildSubagentTools(type,cwd).map(t=>t.name),policies:['headless','interactive','autonomous'].map(mode=>({mode,output:sub.resolveSubagentSafetyPolicy(type,{mode,cwd:'/fixture'})}))});
 }
 const start=new Date('2026-09-06T00:00:00.000Z');
 const manager=new checkpoints.CheckpointManager(cwd,'explore','Inspect the fixture',start);
 manager.addEvidence({kind:'inspect',tool:'read',detail:'src/main.rs'},0);
 manager.addSummaryFragment('Found the entry point',0);
 manager.updateUsage({input:5,output:3,totalTokens:8,cost:0});manager.updateTurn(1);
 manager.finalize('failed');
 const normalize=value=>JSON.parse(JSON.stringify(value,(key,value)=>['startedAt','updatedAt','at'].includes(key)?'2026-09-06T00:00:00.000Z':key==='id'?'<checkpoint>':value));
 checkpointRows.push({state:normalize(manager.getState()),resume:checkpoints.buildResumePrompt(manager.getState(),'Continue inspection')});
 manager.cleanup();
 const broken=new checkpoints.CheckpointManager(cwd,'plan','Retain findings',start);
 const target=path.join(cwd,'.ag1/checkpoints',broken.id+'.json');fs.mkdirSync(target);
 broken.addSummaryFragment('Must remain recoverable',0);broken.finalize('failed');
 fs.writeFileSync(path.join(cwd,'.ag1/outside.json'),JSON.stringify({outside:true}));
 const traversal=checkpoints.readCheckpoint(cwd,'../outside');
 const readonly=sub.validateReadOnlyBashCommand('env touch marker');
 assert.equal(readonly.ok,true);execFileSync('/usr/bin/env',['touch','marker'],{cwd});
 defects={silentWriteFailure:fs.statSync(target).isDirectory()&&broken.getState().finalState==='failed',checkpointTraversal:traversal?.outside===true,readonlyAccepted:readonly.ok,readonlyMutated:fs.existsSync(path.join(cwd,'marker'))};
 assert(Object.values(defects).every(Boolean));
 // finalize flushes synchronously but leaves the existing timer scheduled; cleanup owns it.
 broken.cleanup();
} finally {fs.rmSync(cwd,{recursive:true,force:true});}
const sourceFiles=['src/core/subagent-runner.ts','src/core/checkpoint-manager.ts'];
const captured={version:1,source_commit:'e30f0b4b238ee5618928bcd17b4571eb046e7038',source_files:sourceFiles.map(file=>({path:'packages/coding-agent/'+file,sha256:createHash('sha256').update(fs.readFileSync(path.join(base,file))).digest('hex')})),defaults:{maxTurns:sub.DEFAULT_MAX_TURNS,maxTurnsCap:sub.MAX_TURNS_CAP,timeout:sub.DEFAULT_TIMEOUT_S,tokenBudget:sub.DEFAULT_TOKEN_BUDGET},profiles,checkpointRows,defects,model_actions:0,network_actions:0};
const target=path.join(root,'fixtures/runtime/subagent-oracle.json');
if(process.argv.includes('--capture')){assert(!fs.existsSync(target));fs.writeFileSync(target,JSON.stringify(captured,null,2)+'\n');}
else assert.deepEqual(captured,JSON.parse(fs.readFileSync(target,'utf8')));
console.log(JSON.stringify({status:'passed',profiles:profiles.length,checkpointCases:checkpointRows.length,defects,model_actions:0,network_actions:0}));
