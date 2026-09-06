// Execute the pinned structured planning tool without a session or model.
import fs from 'node:fs';
import path from 'node:path';
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {fileURLToPath,pathToFileURL} from 'node:url';
await import('../offline_guard.mjs');
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../..');
const source='packages/coding-agent/src/core/tools/submit-plan.ts';
const {createSubmitPlanToolDefinition}=await import(pathToFileURL(path.join(root,'reference/ag1-executable/packages/coding-agent/dist/core/tools/submit-plan.js')));
const cases=[
 ['empty',{summary:'Nothing actionable yet.'}],
 ['whitespace',{summary:'  Empty  ',steps:[' ',''],openQuestions:[{question:'  '}]}],
 ['too-many-steps',{summary:'Large',steps:Array.from({length:31},(_,i)=>`Step ${i+1}`)}],
 ['too-many-questions',{summary:'Large',openQuestions:Array.from({length:11},(_,i)=>({question:`Question ${i+1}?`}))}],
 ['too-many-options',{summary:'Choice',openQuestions:[{question:'Pick a language?',options:'abcdefghi'.split('')}]}],
 ['normalized',{summary:'  Normalize me.  ',steps:['  first step  ','','x'+'y'.repeat(600)+'z'],openQuestions:[{question:' Which database? ',options:[' Postgres ','','SQLite']}],assumptions:[' assumption one ',' ']}],
 ['draft',{summary:'Draft plan',steps:['Add endpoint','Wire handler'],openQuestions:[{question:'Transport?',options:['REST','RPC']}],constraints:['Local only'],risks:['Schema mismatch'],verification:['Run contract']}],
 ['clarifying',{summary:'Need a decision',openQuestions:[{question:'Target platform?'}]}],
 ['unicode',{summary:'hé🔍'.repeat(100),steps:['Inspect sources'],verification:['Check result']}],
];
const tool=createSubmitPlanToolDefinition();
const rows=[];
for(const [scenario,args] of cases) rows.push({scenario,args,result:JSON.parse(JSON.stringify(await tool.execute('fixture-call',args,undefined,undefined,undefined)))});
assert.equal(rows.filter(row=>row.result.details?.plan).length,4);
const result={version:1,source_commit:'e30f0b4b238ee5618928bcd17b4571eb046e7038',source_files:[{path:source,sha256:createHash('sha256').update(fs.readFileSync(path.join(root,'reference/ag1-executable',source))).digest('hex')}],rows,model_actions:0,network_actions:0};
const target=path.join(root,'fixtures/runtime/submit-plan-oracle.json');
if(process.argv.includes('--capture')){assert(!fs.existsSync(target));fs.writeFileSync(target,JSON.stringify(result,null,2)+'\n');}
else assert.deepEqual(result,JSON.parse(fs.readFileSync(target,'utf8')));
console.log(JSON.stringify({status:'passed',cases:rows.length,model_actions:0,network_actions:0}));
