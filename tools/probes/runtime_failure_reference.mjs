import fs from 'node:fs';
import path from 'node:path';
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {fileURLToPath,pathToFileURL} from 'node:url';
await import('../offline_guard.mjs');
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../..');
const base=path.join(root,'reference/ag1-executable/packages/coding-agent');
const {AgentSessionToolFailureCoordinator:Tracker}=await import(pathToFileURL(path.join(base,'dist/core/agent-session-tool-failure-coordinator.js')));
const observe=(tool='bash',text='command not found',error=true)=>({kind:'observe',tool,text,error});
const prompt=(active=['bash','read','write'])=>({kind:'prompt',active});
const failures=()=>[observe(),observe(),observe()];
const cases=[
 ['immediate',[observe(),observe()]],
 ['warning-once',[...failures(),prompt(),prompt()]],
 ['success-resets',[...failures(),prompt(),observe('bash','ok',false),...failures(),prompt()]],
 ['explicit-reset',[...failures(),{kind:'reset'},prompt(),observe()]],
 ['unrelated-error',[observe(),observe('bash','permission denied'),observe()]],
 ['exit-code-only',[observe('bash','exited with code 127'),observe('bash','exited with code 127'),prompt()]],
 ['casefold',[observe('bash','COMMAND NOT FOUND'),observe('bash','Command Not Found')]],
 ['insertion-order',[observe('write'),observe('bash'),observe('write'),observe('bash'),observe('write'),observe('bash'),prompt()]],
 ['similar',[observe('readFiles'),observe('readFiles'),observe('readFiles'),prompt(['read','readFile','readFiles','reader','write'])]],
 ['no-active',[...failures(),prompt([])]],
 ['success-reorders',[observe('write'),observe('bash'),observe('write','ok',false),observe('write'),observe('bash'),observe('write'),observe('bash'),observe('write'),prompt()]],
];
const rows=cases.map(([id,steps])=>{
 const tracker=new Tracker(); const outcomes=[];
 for(const step of steps){
  if(step.kind==='reset')tracker.reset();
  else if(step.kind==='prompt'){
   const {message,warnings}=tracker.preparePromptInjection(step.active);
   if(message)delete message.timestamp;
   outcomes.push({kind:'prompt',message:message??null,warnings});
  }else{
   let guidance;
   if(!step.error)tracker.recordSuccess(step.tool);
   else if(step.text.toLowerCase().includes('not found')){
    tracker.recordFailure(step.tool);
    guidance=tracker.prepareImmediateResultGuidance(step.tool,step.text);
   }
   outcomes.push({kind:'observe',guidance:guidance??null});
  }
 }
 return {id,steps,outcomes};
});
const source='packages/coding-agent/src/core/agent-session-tool-failure-coordinator.ts';
const result={version:1,source_commit:'e30f0b4b238ee5618928bcd17b4571eb046e7038',source_files:[{path:source,sha256:createHash('sha256').update(fs.readFileSync(path.join(base,'src/core/agent-session-tool-failure-coordinator.ts'))).digest('hex')}],rows,model_actions:0,network_actions:0};
const target=path.join(root,'fixtures/runtime/failure-oracle.json');
if(process.argv.includes('--capture')){assert(!fs.existsSync(target));fs.writeFileSync(target,JSON.stringify(result,null,2)+'\n');}
else assert.deepEqual(result,JSON.parse(fs.readFileSync(target,'utf8')));
console.log(JSON.stringify({status:'passed',cases:rows.length,model_actions:0,network_actions:0}));
