// Pure state contracts from the immutable SDK. No workspace mutation or model call.
import fs from 'node:fs';
import path from 'node:path';
import assert from 'node:assert/strict';
import {fileURLToPath,pathToFileURL} from 'node:url';
await import('../offline_guard.mjs');
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../..');
const mod=async name=>import(pathToFileURL(path.join(root,`reference/ag1-executable/packages/coding-agent/dist/core/${name}.js`)));
const task=await mod('task-state'),memory=await mod('memory-store'),selection=await mod('memory-context'),todo=await mod('todos'),runtime=await mod('runtime-observations'),inspection=await mod('inspection-evidence');
const now='2026-09-05T00:00:00.000Z', rows=[];
function capture(kind,input,fn){let output;try{output={ok:true,value:fn()}}catch{output={ok:false}}rows.push({kind,input,output});}
let state=task.setTaskState(undefined,{objective:' Build  desktop ',latestInstruction:'start',constraints:['safe','safe',' local ']},now);
capture('task_set',{state:null,patch:{objective:' Build  desktop ',latestInstruction:'start',constraints:['safe','safe',' local ']},now},()=>state);
for(const patch of [{latestInstruction:'fix the settings'},{currentPhase:' phase seven ',nextAction:'read evidence'},{status:'wrong'},{objective:' '},{blockerId:'ts_00000001'},{attemptIds:['missing']}])capture('task_set',{state,patch,now},()=>task.setTaskState(state,patch,now));
for(const text of ['  hello \n world ','<skill name="x">do not use as objective</skill>\n real task','<skill name="x">unclosed','<skill a="b">x</skill><skill c="d">y</skill>  next '])capture('canonical_task',{text},()=>task.canonicalTaskText(text)??null);
const entries=Array.from({length:8},(_,i)=>({id:`ts_0000000${i}`,kind:['FACT','OBSERVATION','HYPOTHESIS','DECISION','CONSTRAINT','ASSUMPTION','RESULT','FAILURE'][i],statement:' entry  '+i,status:'active',sources:['tool-call:c1'],createdAt:now,updatedAt:now}));
// Exercise the persisted-state boundary before subsequent mutations.
state=task.parseTaskStateSnapshot({...state,entries});
for(const patch of [{blockerId:entries[7].id,status:'blocked'},{checkpointId:entries[6].id},{attemptIds:entries.map(e=>e.id)},{checkpointId:entries[0].id}])capture('task_set',{state,patch,now},()=>task.setTaskState(state,patch,now));
for(const patch of [{status:'resolved'},{kind:'HYPOTHESIS',sources:[]},{kind:'FACT',sources:[]},{statement:'  revised ',confidence:'high'},{status:'wrong'}])capture('task_update',{state:{...state,status:'blocked',blockerId:entries[7].id},id:entries[7].id,patch,now},()=>task.updateTaskStateEntry({...state,status:'blocked',blockerId:entries[7].id},entries[7].id,patch,now));
for(const suffix of [[],[{type:'message',id:'e2',message:{role:'toolResult',toolName:'bash',toolCallId:'c2',isError:false}}],[{type:'message',id:'e2',message:{role:'toolResult',toolName:'read',toolCallId:'c2',isError:false}}],[{type:'message',id:'e2',message:{role:'toolResult',toolName:'check',toolCallId:'c2',details:{passed:false}}}]]){const journal=[{type:'message',id:'e1',message:{role:'toolResult',toolName:'check',toolCallId:'c1',details:{passed:true},isError:false}},...suffix];capture('passing_check',{sources:['tool-call:c1'],journal},()=>task.hasRecordedPassingCheck(['tool-call:c1'],journal));}
let store=memory.emptyMemoryStore();
for(let i=0;i<45;i++){const input={store,kind:i%2?'fact':'decision',text:`entry ${i} ${i%3?'ordinary':'source compiler'}`,now:Date.UTC(2026,8,5)+i,priority:i<2?'core':'normal',metadata:i%7?{}:{component:'compiler',sources:['file:a.ts@sha256:'+'a'.repeat(64)]}};capture('memory_add',input,()=>memory.addMemoryEntry(input.store,input.kind,input.text,input.now,input.priority,input.metadata));store=memory.addMemoryEntry(input.store,input.kind,input.text,input.now,input.priority,input.metadata).store;}
for(const patch of [{text:'updated compiler'},{priority:'core'},{sources:[],component:'compiler'},{component:''},{}])capture('memory_update',{store,id:'m2',patch,now:100000},()=>memory.updateMemoryEntry(store,'m2',patch,100000));
for(const query of ['', 'compiler source', 'ordinary'])for(const status of ['current','stale','unverified'])capture('memory_select',{store,query,status},()=>selection.selectMemoriesForContext(store,{query,validateSources:()=>status}));
for(const items of [[],[{title:' first ',status:'in_progress'},{title:'next'}],[{title:'x',id:'t1'},{title:'y',id:'t1'}],[{title:'x',status:'in_progress'},{title:'y',status:'in_progress'}],[{title:''}],null])capture('todo_validate',{items},()=>{const r=todo.validateTodoItems(items);return r.ok?{ok:true,items:r.items}:{ok:false}});
for(const value of [{version:1,id:'obs_012345abcdef',feature:' UI ',result:'pass',environment:'linux',revision:'target:r1',reporter:'user',reproduction:'open settings',notes:'theme works',sources:['artifact:a'],observedAt:now},{version:1,id:'obs_bad'}])capture('runtime_parse',{value},()=>runtime.parseRuntimeObservation(value)??null);
for(const scope of [{startLine:1,endLine:5},{startLine:0,endLine:1},{startLine:8,endLine:5}]){const value={version:1,kind:'read_range',source:'file:a.ts@sha256:'+'a'.repeat(64),toolCallId:'read1',scope};capture('inspection_parse',{value},()=>inspection.parseInspectionEvidence(value)??null);}
const out=JSON.parse(JSON.stringify({version:1,source_commit:'e30f0b4b238ee5618928bcd17b4571eb046e7038',model_actions:0,network_actions:0,rows}));
const target=path.join(root,'fixtures/evidence/state-oracle-v2.json');
if(process.argv.includes('--capture')){assert(!fs.existsSync(target));fs.writeFileSync(target,JSON.stringify(out,null,2)+'\n')}else assert.deepEqual(out,JSON.parse(fs.readFileSync(target,'utf8')));
console.log(JSON.stringify({status:'passed',cases:rows.length,model_actions:0,network_actions:0}));
