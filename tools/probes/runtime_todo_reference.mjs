// Actual SDK todo tool and AgentSession lifecycle preflight, with journal projections
// using the unchanged exported snapshot/archive functions consumed by its after-hook.
import fs from 'node:fs';import path from 'node:path';import assert from 'node:assert/strict';import {createHash} from 'node:crypto';import {fileURLToPath,pathToFileURL} from 'node:url';
await import('../offline_guard.mjs');
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../..');
const base=path.join(root,'reference/ag1-executable/packages/coding-agent');
const {createTodoToolDefinition}=await import(pathToFileURL(path.join(base,'dist/core/tools/todo.js')));
const {AgentSession}=await import(pathToFileURL(path.join(base,'dist/core/agent-session.js')));
const todo=await import(pathToFileURL(path.join(base,'dist/core/todos.js')));
const now=1788652800000,newPlanId='plan_000000000001';
const items=[{id:'t1',title:'Inspect source',status:'in_progress'},{id:'t2',title:'Implement changes',status:'pending'}];
const active={version:1,items,updatedAt:now,planId:'plan_000000000000',phase:'Existing phase'};
const completed={...active,items:items.map(i=>({...i,status:'completed'}))};
const cases=[
 {id:'query',args:{}},
 {id:'query-phase',state:active,args:{phase:'ignored query phase'}},
 {id:'create',args:{todos:[{title:'  Inspect\n source ',status:'in_progress'},{title:'Implement changes'}],phase:' Native phase '}},
 {id:'advance',state:active,args:{todos:[{...items[0],status:'completed'},{...items[1],status:'in_progress'}]}},
 {id:'complete',state:active,args:{todos:completed.items}},
 {id:'new-after-complete',state:completed,args:{todos:[{title:'Start another task',status:'in_progress'}]}},
 {id:'clear-complete',state:completed,args:{clear:true}},
 {id:'clear-empty',args:{clear:true}},
 {id:'clear-active',state:active,args:{clear:true}},
 {id:'archive-active',state:active,args:{planAction:'archive'}},
 {id:'archive-empty',args:{planAction:'archive'}},
 {id:'new-plan',state:active,args:{planAction:'new-plan',phase:' Next phase ',todos:[{title:'New work',status:'in_progress'}]}},
 {id:'invalid-retrospective',args:{todos:[{title:'Already done',status:'completed'}]}},
 {id:'invalid-no-active',args:{todos:[{title:'No active step'}]}},
 {id:'invalid-disappear',state:active,args:{todos:[items[0]]}},
 {id:'invalid-rename',state:active,args:{todos:[{...items[0],title:'Changed title'},items[1]]}},
 {id:'invalid-clear-mixed',state:completed,args:{clear:true,todos:[]}},
 {id:'invalid-archive-phase',state:active,args:{planAction:'archive',phase:''}},
 {id:'invalid-new-plan',state:active,args:{planAction:'new-plan'}},
];
const rows=[];
for(const test of cases) {
 const branch=test.state?[{type:'custom',customType:'todo_list',data:structuredClone(test.state)}]:[];
 const records=[];
 const manager={getBranch:()=>branch,appendCustomEntryAsync:async(customType,data)=>{const record={type:'custom',customType,data};records.push(record);branch.push(record);return 'owned-entry';}};
 const blocked=AgentSession.prototype._evaluateTodoLifecycle.call({sessionManager:manager},'todo',test.args);
 let output,error;
 try {
  if(blocked?.block) throw new Error(blocked.reason);
  output=await createTodoToolDefinition().execute('owned-todo',test.args);
  const current=todo.readLatestTodoSnapshot(branch);const details=output.details;let snapshot;
  if(details.planAction==='archive'||details.planAction==='new-plan') {
   if(current?.items.length) await todo.appendTodoArchiveAsync(manager,current,details.planAction,now);
   snapshot=details.planAction==='archive'?todo.clearTodoList(now):todo.replaceTodoList(null,todo.validateTodoItems(details.todos).items,now,{planId:newPlanId,phase:details.phase});
  } else if(details.cleared===true) snapshot=todo.clearTodoList(now);
  else if(Array.isArray(details.todos)) snapshot=todo.replaceTodoList(current,todo.validateTodoItems(details.todos).items,now,{phase:details.phase});
  if(snapshot) await todo.appendTodoSnapshotAsync(manager,snapshot);
 } catch(caught){error=true;}
 rows.push({...test,...(error?{error}:{output}),records});
}
const result={version:1,source_commit:'e30f0b4b238ee5618928bcd17b4571eb046e7038',source_files:['tools/todo','todos','agent-session'].map(name=>({path:`packages/coding-agent/src/core/${name}.ts`,sha256:createHash('sha256').update(fs.readFileSync(path.join(base,`src/core/${name}.ts`))).digest('hex')})),now,newPlanId,rows,model_actions:0,network_actions:0};
const target=path.join(root,'fixtures/runtime/todo-tool-oracle.json');
if(process.argv.includes('--capture')){assert(!fs.existsSync(target));fs.writeFileSync(target,JSON.stringify(result,null,2)+'\n');}else assert.deepEqual(JSON.parse(JSON.stringify(result)),JSON.parse(fs.readFileSync(target,'utf8')));
console.log(JSON.stringify({status:'passed',todo_cases:rows.length,model_actions:0,network_actions:0}));
