import fs from 'node:fs';
import path from 'node:path';
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {fileURLToPath,pathToFileURL} from 'node:url';
await import('../offline_guard.mjs');
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../..');
const base=path.join(root,'reference/ag1-executable/packages/coding-agent');
const module=async name=>import(pathToFileURL(path.join(base,'dist/core',name+'.js')));
const {classifyVerificationFailure}=await module('verification-controller');
const {RepairController}=await module('repair-controller');
const {StopController,decideStop}=await module('stop-controller');
const assertion='not ok 1 - saves normalized scope\nAssertionError [ERR_ASSERTION]: Expected values to be strictly deep-equal\nactual: []\nexpected: admin\nstack: tests/session-store.test.mjs:14:10';
const outputs=[
 '', 'TAP version 13\n---\nerror: |-\n1..3\n# tests 3\n# pass 0\n# fail 3', assertion,
 '\x1b[31mSyntaxError: Unexpected token at src/parser.js:4\x1b[0m\r\n',
 'Error [ERR_MODULE_NOT_FOUND]: Cannot find module ./missing.js',
 'Command timed out after 30000ms', 'time out now', 'timeout network SyntaxError AssertionError',
 'Cannot find module ./missing.js SyntaxError AssertionError',
 'SyntaxError network AssertionError', 'network error AssertionError',
 'EACCES: permission denied', 'spawn /not-here ENOENT', 'ENOENT open src/missing.ts',
 'Address already in use', 'Connection refused', 'not ok 1 tests/unrelated.test.mjs AssertionError',
 'at /tmp/work/src/parser.js:4:5 AssertionError', 'at ./src/parser.js:4:5 AssertionError',
 'src/parser.js src/parser.js src/other.ts node:internal.js file:///tmp/a.js',
 'src/widget.tsx:4 src/component.jsx:3 tests/store.test.mjs:14:10',
 'TS1234: unknown signature', 'Unexpected token', 'unknown failure',
 'before\nAssertionError duplicate\nAssertionError duplicate\nAssertionError duplicate\nsecond\nthird\nfourth',
 'ordinary first\nexpected something\nordinary second\nError: use this\nordinary last',
 '\ufeffError: fullwidth １２ and ASCII 12 at src/parser.js:14:10\ufeff',
 'Error: emoji '+ '😀'.repeat(100),
 'Error: '+ 'long '.repeat(130),
 'not okay (not a boundary)\nnot ok!',
];
const relevantSets=[[],['src/parser.js','tests/session-store.test.mjs']];
const failures=outputs.flatMap(output=>relevantSets.map(relevantPaths=>({output,relevantPaths,result:classifyVerificationFailure(output,{relevantPaths})})));
const repairs=[];
const repairCases=[
 {options:{},actions:[['failure',assertion],['mutation'],['failure',assertion.replace('14:10','22:11')],['mutation'],['success']]},
 {options:{relevantPaths:['tests/a.test.js','src/parser.js'],expectedChangedPaths:['src/target.js'],repeatedFailureLimit:2},actions:[['mutation'],['failure',assertion],['failure',assertion],['success'],['failure','SyntaxError: src/parser.js:5'],['mutation'],['failure','Error: network unavailable']]},
 {options:{relevantPaths:['tests/a.test.js']},actions:[['failure','timeout'],['failure','timeout'],['failure','timeout']]},
 {options:{relevantPaths:['src/a.js'],repeatedFailureLimit:3},actions:[['failure',assertion],['mutation'],['failure',assertion],['mutation'],['failure',assertion],['success']]},
 {options:{expectedChangedPaths:[''],relevantPaths:['src/a.js']},actions:[['failure','Error: failed']]},
];
for(const {options,actions} of repairCases){const c=new RepairController(options);const observations=[];for(const action of [['initial'],...actions]){let result=null;if(action[0]==='failure')result=c.noteVerificationFailure(action[1]);if(action[0]==='success')c.noteVerificationSuccess();if(action[0]==='mutation')c.noteMutation();observations.push({action,result,snapshot:c.snapshot(),decision:c.evaluateVerifierRun(),guidance:c.getGuidance()??null,next:c.nextRepairAction()});}repairs.push({options,observations});}
const stopInputs=[
 {intent:'debug_fix'}, {intent:'debug_fix',verificationPassedAfterMutation:true},
 {intent:'debug_fix',verificationPassedAfterMutation:true,stopAfterVerifierPass:false},
 {intent:'direct_answer',sufficientEvidence:true}, {intent:'code_explanation',sufficientEvidence:true},
 {intent:'research',sourceCount:2}, {intent:'research',sourceCount:3},
 {intent:'debug_fix',budgetExhausted:true}, {intent:'debug_fix',repeatedFailureCount:2},
 {intent:'debug_fix',repeatedFailureCount:3}, {intent:'debug_fix',repeatedFailureCount:1,repeatedFailureLimit:1},
 {intent:'debug_fix',repeatedFailureLimit:0},
 {intent:'research',sourceCount:4,budgetExhausted:true,verificationPassedAfterMutation:true},
 {intent:'direct_answer',sufficientEvidence:true,budgetExhausted:true},
 {intent:'research',sourceCount:3,budgetExhausted:true},
 {intent:'debug_fix',budgetExhausted:true,repeatedFailureCount:3},
];
const stops=[true,false].map(stopAfterVerifierPass=>{const c=new StopController({stopAfterVerifierPass});const actions=[['initial'],['verify',true,true],['mutation'],['verify',false,true],['verify',true,false],['verify',true,true],['verify',true,false],['mutation'],['verify',true,false],['mutation'],['verify',true,true]];return {stopAfterVerifierPass,observations:actions.map(action=>{if(action[0]==='mutation')c.noteMutation();if(action[0]==='verify')c.noteVerification({matchedVerifier:action[1],success:action[2]});return {action,snapshot:c.snapshot(),decision:c.evaluateToolUse()};})};});
const source_files=['verification-controller.ts','repair-controller.ts','stop-controller.ts'].map(name=>({path:'packages/coding-agent/src/core/'+name,sha256:createHash('sha256').update(fs.readFileSync(path.join(base,'src/core',name))).digest('hex')}));
const result={version:1,source_commit:'e30f0b4b238ee5618928bcd17b4571eb046e7038',source_files,failures,repairs,decisions:stopInputs.map(input=>({input,result:decideStop(input)})),stops,model_actions:0,network_actions:0};
const target=path.join(root,'fixtures/runtime/policy-control-oracle.json');
if(process.argv.includes('--capture')){assert(!fs.existsSync(target));fs.writeFileSync(target,JSON.stringify(result,null,2)+'\n');}
// JSON omits undefined optional fields; compare the same wire shape captured above.
else assert.deepEqual(JSON.parse(JSON.stringify(result)),JSON.parse(fs.readFileSync(target,'utf8')));
console.log(JSON.stringify({status:'passed',failures:failures.length,repairTraces:repairs.length,stopDecisions:stopInputs.length,stopTraces:stops.length,model_actions:0,network_actions:0}));
