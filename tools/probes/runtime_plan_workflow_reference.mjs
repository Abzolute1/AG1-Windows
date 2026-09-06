// Execute the pinned plan constructors and step review reducers without a UI.
import fs from 'node:fs';
import path from 'node:path';
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {fileURLToPath,pathToFileURL} from 'node:url';
await import('../offline_guard.mjs');
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../..');
const source='packages/coding-agent/src/modes/interactive/interactive-plan-artifacts.ts';
const ref=await import(pathToFileURL(path.join(root,'reference/ag1-executable/packages/coding-agent/dist/modes/interactive/interactive-plan-artifacts.js')));
const now='2026-09-06T00:00:00.000Z';
const fixed=plan=>plan?{...plan,planId:'plan-a'}:null;
const submissions=[
  {goal:'Add endpoint',submission:{steps:['Inspect inputs','Implement endpoint'],summary:'Local API'}},
  {goal:'Build a desktop editor',submission:{steps:[' Inspect  source\ncarefully ','Ship'],openQuestions:[{question:' Target  OS? ',options:[' Linux ','  ',' Windows ']}],assumptions:[' Already\nselected '],constraints:['Offline'],risks:['Compatibility'],verification:['Run tests']}},
  {goal:'  Plan out the API  ',submission:{summary:' Need a choice ',openQuestions:[{question:'Platform?',options:['Web','Native']},{question:'   '}]}},
  {goal:'Inspect Rust 🦀',submission:{steps:['  café\n🔍  ']}},
  {goal:'Empty',submission:{}},
].map(input=>({input,result:fixed(ref.buildInteractivePlanArtifactFromSubmission({...input,now}))}));
const draft=submissions[0].result;
const previous=ref.setInteractivePlanClarificationAnswer(submissions[2].result,0,'Native',now);
submissions.push({input:{goal:'the API',submission:{steps:['Implement selected platform']},previous,feedback:'  Keep it local  '},result:fixed(ref.buildInteractivePlanArtifactFromSubmission({goal:'the API',submission:{steps:['Implement selected platform']},previous,feedback:'  Keep it local  ',now}))});
const texts=[
  '', 'Done.', 'Execution Status:\ncompleted',
  'Execution Status:\ncompleted\nExecution Summary:\nImplemented **endpoint** and `tests`.\nFiles Touched:\n- src/api.rs\nVerification:\n- cargo test\nRemaining:\n- None\nBlockers:\n- N/A',
  '# Execution Status\nIn progress\n## Step Summary\nPartial work\n### Remaining Work\n* Wire the handler\n### Issues\n- no blockers',
  'Execution Status:\nblocked\nReview Summary:\nMissing API key\nBlockers:\n- Credential unavailable',
  'Execution Status:\nunknown\nExecution Summary:\nAwaiting decision\nIssues:\n- Need target',
  'Execution Status:\nunknown\nExecution Summary:\nNo status evidence',
];
const reviews=texts.map(text=>({text,result:ref.parseInteractivePlanStepExecutionReview(text,0,draft.steps[0],now)??null}));
const accepted=ref.updateInteractivePlanArtifactStatus(draft,'accepted',now);
const executing=ref.updateInteractivePlanArtifactStatus(accepted,'executing',now,{executionStepIndex:0,executionStep:accepted.steps[0]});
const marked=ref.markInteractivePlanStepComplete(executing,0,now);
const completed=ref.markInteractivePlanStepComplete(marked,1,now);
const reopened=ref.reopenInteractivePlanStep(completed,0,now);
const reviewed=ref.recordInteractivePlanStepExecutionReview(executing,0,texts[3],now);
const entry=data=>({type:'custom',customType:'interactive_plan_artifact',data});
const observed=ref.getLatestInteractivePlanArtifact([entry(executing),entry(reviewed)]);
assert.equal(reviewed.status,'executing');
assert.equal(reviewed.executionStepIndex,undefined);
assert.equal(observed.executionStepIndex,0);
assert.equal(ref.getCompletedStepCount(observed),0);
const result=JSON.parse(JSON.stringify({version:1,source_commit:'e30f0b4b238ee5618928bcd17b4571eb046e7038',source_files:[{path:source,sha256:createHash('sha256').update(fs.readFileSync(path.join(root,'reference/ag1-executable',source))).digest('hex')}],submissions,reviews,executing,marked,completed,reopened,reviewed,observedAfterReview:observed,model_actions:0,network_actions:0}));
const target=path.join(root,'fixtures/runtime/plan-workflow-oracle.json');
if(process.argv.includes('--capture')){assert(!fs.existsSync(target));fs.writeFileSync(target,JSON.stringify(result,null,2)+'\n');}
else assert.deepEqual(result,JSON.parse(fs.readFileSync(target,'utf8')));
console.log(JSON.stringify({status:'passed',submissions:submissions.length,reviews:reviews.length,confirmed_review_regression:true,model_actions:0,network_actions:0}));
