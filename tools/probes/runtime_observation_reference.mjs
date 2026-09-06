// Actual unchanged observation tool with a disposable journal port shared with task state.
import fs from 'node:fs';import path from 'node:path';import assert from 'node:assert/strict';import {createHash} from 'node:crypto';import {fileURLToPath,pathToFileURL} from 'node:url';
await import('../offline_guard.mjs');
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../..');
const base=path.join(root,'reference/ag1-executable/packages/coding-agent');
const {createRuntimeObservationToolDefinition}=await import(pathToFileURL(path.join(base,'dist/core/tools/runtime-observation.js')));
const now='2026-09-06T00:00:00.000Z';
const task={version:1,status:'completed',objective:'Repair compiler startup',constraints:[],openQuestions:[],entries:[],updatedAt:now};
const input={feature:'Compiler startup',result:'fail',environment:'owned fixture',revision:'workspace:fixture',reporter:'user',reproduction:'Open the test view',notes:'Startup stops before rendering'};
const cases=[
 {id:'no-task-failure',args:input},
 {id:'no-task-pass',args:{...input,result:'pass',reporter:'agent'}},
 {id:'related-failure',state:task,args:input},
 {id:'related-pass',state:task,args:{...input,result:'pass',reporter:'verifier',sources:['artifact:owned-test']}},
 {id:'related-partial',state:task,args:{...input,result:'partial'}},
 {id:'related-blocked',state:task,args:{...input,result:'blocked'}},
 {id:'unrelated-failure',state:task,args:{...input,feature:'Animation timing'}},
 {id:'phase-related',state:{...task,objective:'Other task',currentPhase:'Compiler diagnosis'},args:input},
 {id:'question-related',state:{...task,objective:'Other task',openQuestions:['Does compiler startup complete?']},args:input},
 {id:'short-feature',state:task,args:{...input,feature:'UI'}},
 {id:'empty-sources',state:task,args:{...input,sources:[]}},
 {id:'invalid-empty',state:task,args:{...input,notes:'   '}},
];
const OriginalDate=globalThis.Date,rows=[];
try {
 globalThis.Date=class extends OriginalDate{constructor(...args){super(...(args.length?args:[now]));}static now(){return new OriginalDate(now).getTime();}};
 for(const test of cases) {
  const branch=test.state?[{type:'custom',customType:'task_state',data:structuredClone(test.state)}]:[];
  const records=[];
  const manager={getBranch:()=>branch,appendCustomEntryAsync:async(customType,data)=>{const id=customType==='runtime_observation'?'observation-entry':'task-entry';const row={type:'custom',customType,data};records.push(row);branch.push(row);return id;}};
  let output,error;
  try{output=await createRuntimeObservationToolDefinition({sessionManager:manager,taskState:manager}).execute('observation-call',test.args);}catch(caught){error=true;}
  let row={...test,...(error?{error}:{output}),records};
  if(output){let text=JSON.stringify(row).replaceAll(output.details.id,'obs_000000000001');if(output.details.taskStateEntryId)text=text.replaceAll(output.details.taskStateEntryId,'ts_00000001');row=JSON.parse(text);}
  rows.push(row);
 }
}finally{globalThis.Date=OriginalDate;}
const result={version:1,source_commit:'e30f0b4b238ee5618928bcd17b4571eb046e7038',source_files:['tools/runtime-observation','runtime-observations','task-state'].map(name=>({path:`packages/coding-agent/src/core/${name}.ts`,sha256:createHash('sha256').update(fs.readFileSync(path.join(base,`src/core/${name}.ts`))).digest('hex')})),now,rows,model_actions:0,network_actions:0};
const target=path.join(root,'fixtures/runtime/observation-tool-oracle.json');
if(process.argv.includes('--capture')){assert(!fs.existsSync(target));fs.writeFileSync(target,JSON.stringify(result,null,2)+'\n');}else assert.deepEqual(JSON.parse(JSON.stringify(result)),JSON.parse(fs.readFileSync(target,'utf8')));
console.log(JSON.stringify({status:'passed',observation_cases:rows.length,model_actions:0,network_actions:0}));
