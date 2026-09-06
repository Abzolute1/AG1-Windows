// Pure pinned task-policy/evidence-planner captures; files live only in owned tempdirs.
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {fileURLToPath,pathToFileURL} from 'node:url';
await import('../offline_guard.mjs');
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../..');
const base=path.join(root,'reference/ag1-executable/packages/coding-agent');
const module=async name=>import(pathToFileURL(path.join(base,'dist/core',name+'.js')));
const {buildWorkspaceIndex}=await module('workspace-index');
const {inferTaskPolicy,classifyControlledTaskIntent,formatTaskPolicy}=await module('task-policy');
const {buildEvidencePlan,nextEvidencePlanStep,formatEvidencePlanStep}=await module('evidence-planner');
const prompts=[
 '', ' \ufeff\t', 'Fix the parser bug', 'Add an option', 'Refactor this across modules',
 'Explain how the parser works', 'Research the latest API release', 'Improve the responsive UI component',
 'Run tests only', 'Which file defines the config parser? Give the exact path.', 'Something undecided',
 'Postmortem only. Do not use tools and do not edit files. The repair was unsuccessful; explain why.',
 'Just inspect this UI; do not change anything.',
 'Perform a rigorous read-only cockpit audit with evidence and no source or tracked-file edits.',
 'Update the parser to the latest supported API', 'Review current behavior only; do not edit files.',
 "Never modify any source files. Explain the fix.", "Don't run any tools. Repair the bug.",
 'Only look at the patch; dont act.', 'read-only cockpit audit only: fix this',
 'Analysis only: implement a fix', 'Inspection only: fix the bug', 'review only: fix this',
 'Improve CSS across all files', 'Run tests and fix the failure', 'Search online how does this work',
 'Migration across the codebase', 'Migrate across the codebase', 'hi fix', 'Run targeted verification',
 ...['éfix','fixé','αfixβ','修fix正','_fix_','ſix','fİx','checK','do not\ufeffedit files. fix it','do not\u0085edit files. fix it','do not edit naïve files. fix it','do not edit naive files. fix it'],
];
const classifications=prompts.map(prompt=>({prompt,intent:classifyControlledTaskIntent(prompt)}));
const fixtures=JSON.parse(fs.readFileSync(path.join(root,'fixtures/runtime/verifier-oracle.json'),'utf8')).rows.map(row=>row.input);
fixtures.push({id:'policy-targets',files:{'package.json':'{"scripts":{"test":"node --test"}}','README.md':'','src/config-parser.mjs':"import './defaults.mjs';",'src/defaults.mjs':'','src/extra.ts':'','tests/config-parser.test.mjs':"import '../src/config-parser.mjs';",'generated/parser.ts':'','src/parser.snap.ts':'','src/PARSER.TS':'','src/parse.css':'','src/parse.html':'','src/parser.rs':''},prompts:prompts.slice(0,11)});
const rows=[];
for(const input of fixtures){
 const cwd=fs.mkdtempSync(path.join(os.tmpdir(),'ag1-policy-oracle-'));
 try{
  const files={...input.files};
  for(let i=0;i<(input.bulk??0);i++)files['aaa/file-'+String(i).padStart(4,'0')+'.txt']='';
  for(let i=0;i<(input.testBulk??0);i++)files['test/'+String(i).padStart(2,'0')+'.test.js']="import '../src/target.js';";
  for(const [name,bytes] of Object.entries(files)){const target=path.join(cwd,name);fs.mkdirSync(path.dirname(target),{recursive:true});fs.writeFileSync(target,bytes);}
  const index=buildWorkspaceIndex({rootDir:cwd,useCache:false});
  for(const prompt of input.prompts??[input.prompt??'Fix the implementation.'])for(const mode of ['flexible','controlled','strict_benchmark']){
   const policy=inferTaskPolicy({prompt,index,mode});
   const plan=buildEvidencePlan(policy);
   const states=[{}, {readPaths:policy.likelyTests}, {readPaths:policy.allowedPaths}, {readPaths:policy.allowedPaths,mutatedPaths:policy.expectedChangedPaths,mutationSinceVerifier:true}, {verifierPassedAfterMutation:true}];
   rows.push({id:input.id,prompt,mode,index:{files:index.files.map(f=>f.path),packageScripts:index.packageScripts,sources:files},policy,formatted:formatTaskPolicy(policy),plan,steps:plan.map(formatEvidencePlanStep),next:states.map(state=>({state,step:nextEvidencePlanStep(plan,state)??null}))});
  }
 }finally{fs.rmSync(cwd,{recursive:true,force:true});}
}
const source_files=['task-policy.ts','workspace-index.ts','verification-controller.ts','evidence-planner.ts'].map(name=>({path:'packages/coding-agent/src/core/'+name,sha256:createHash('sha256').update(fs.readFileSync(path.join(base,'src/core',name))).digest('hex')}));
const result={version:1,source_commit:'e30f0b4b238ee5618928bcd17b4571eb046e7038',source_files,classifications,rows,model_actions:0,network_actions:0};
const target=path.join(root,'fixtures/runtime/policy-oracle.json');
if(process.argv.includes('--capture')){assert(!fs.existsSync(target));fs.writeFileSync(target,JSON.stringify(result,null,2)+'\n');}
// JSON omits undefined optional fields; compare the same wire shape captured above.
else assert.deepEqual(JSON.parse(JSON.stringify(result)),JSON.parse(fs.readFileSync(target,'utf8')));
console.log(JSON.stringify({status:'passed',classifications:classifications.length,policies:rows.length,model_actions:0,network_actions:0}));
