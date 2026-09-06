// Real task-state tool and reducers, with a disposable in-memory journal port.
import fs from 'node:fs';import path from 'node:path';import assert from 'node:assert/strict';import {createHash} from 'node:crypto';import {fileURLToPath,pathToFileURL} from 'node:url';
await import('../offline_guard.mjs');
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../..');
const base=path.join(root,'reference/ag1-executable/packages/coding-agent');
const {createTaskStateToolDefinition}=await import(pathToFileURL(path.join(base,'dist/core/tools/task-state.js')));
const now='2026-09-06T00:00:00.000Z';
const initial={version:1,status:'active',objective:'Native task',constraints:[],openQuestions:[],entries:[],updatedAt:now};
const entry={id:'ts_00000001',kind:'HYPOTHESIS',statement:'Investigate ownership',status:'active',confidence:'medium',sources:[],createdAt:now,updatedAt:now};
const withEntry={...initial,entries:[entry]};
const cases=[
 {id:'empty-show',args:{action:'show'}},
 {id:'show',state:initial,args:{action:'show'}},
 {id:'initialize',args:{action:'set',objective:'Initialize task',currentSubgoal:'Inspect source'}},
 {id:'reset',state:withEntry,args:{action:'reset',objective:'New task',currentPhase:'Implementation',constraints:['Preserve tuning']}},
 {id:'set',state:initial,args:{action:'set',currentPhase:'Native tools',nextAction:'Run checks',openQuestions:['Does cancellation drain?']}},
 {id:'record',state:initial,args:{action:'record',kind:'DECISION',statement:'Use the session owner',reason:'Preserve acknowledgement ordering'}},
 {id:'duplicate',state:withEntry,args:{action:'record',kind:'HYPOTHESIS',statement:'Investigate ownership'}},
 {id:'update',state:withEntry,args:{action:'update',id:entry.id,statement:'Ownership verified',entryStatus:'resolved',confidence:'high'}},
 {id:'remove',state:withEntry,args:{action:'remove',id:entry.id}},
 {id:'invalid-fact',state:initial,args:{action:'record',kind:'FACT',statement:'Unsupported claim'}},
 {id:'invalid-set',state:initial,args:{action:'set'}},
 {id:'invalid-empty',args:{action:'record',kind:'HYPOTHESIS',statement:'No task'}},
];
const OriginalDate=globalThis.Date;const rows=[];
try {
 globalThis.Date=class extends OriginalDate {constructor(...args){super(...(args.length?args:[now]));}static now(){return new OriginalDate(now).getTime();}};
 for(const test of cases) {
  const branch=test.state?[{type:'custom',customType:'task_state',data:structuredClone(test.state)}]:[];
  const manager={getBranch:()=>branch,appendCustomEntryAsync:async(customType,data)=>{branch.push({type:'custom',customType,data});return 'owned-entry';}};
  let output,error;
  try {output=await createTaskStateToolDefinition({sessionManager:manager}).execute('owned-call',test.args);}catch(caught){error=String(caught);}
  if(test.id==='record') {
   const id=output.details.entryId;
   output=JSON.parse(JSON.stringify(output).replaceAll(id,'ts_00000002'));
   branch[branch.length-1]=JSON.parse(JSON.stringify(branch[branch.length-1]).replaceAll(id,'ts_00000002'));
  }
  // Pretty JSON ordering is not semantic; compare the decoded show document.
  if(output && test.args.action==='show' && test.state) output.content[0].text=JSON.parse(output.content[0].text);
  rows.push({...test,...(error?{error:true}:{output}),after:branch.length?branch[branch.length-1].data:null,writes:branch.length-(test.state?1:0)});
 }
}finally{globalThis.Date=OriginalDate;}
const result={version:1,source_commit:'e30f0b4b238ee5618928bcd17b4571eb046e7038',source_files:['task-state','tools/task-state'].map(name=>({path:`packages/coding-agent/src/core/${name}.ts`,sha256:createHash('sha256').update(fs.readFileSync(path.join(base,`src/core/${name}.ts`))).digest('hex')})),now,rows,model_actions:0,network_actions:0};
const target=path.join(root,'fixtures/runtime/task-state-tool-oracle.json');
if(process.argv.includes('--capture')){assert(!fs.existsSync(target));fs.writeFileSync(target,JSON.stringify(result,null,2)+'\n');}else assert.deepEqual(JSON.parse(JSON.stringify(result)),JSON.parse(fs.readFileSync(target,'utf8')));
console.log(JSON.stringify({status:'passed',task_state_cases:rows.length,model_actions:0,network_actions:0}));
