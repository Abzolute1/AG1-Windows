import fs from 'node:fs';
import path from 'node:path';
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {fileURLToPath,pathToFileURL} from 'node:url';
await import('../offline_guard.mjs');
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../..');
const base=path.join(root,'reference/ag1-executable/packages/coding-agent');
const sdk=await import(pathToFileURL(path.join(base,'dist/core/coding-completion-gate.js')));
const commands=['pytest tests -q','python3.12 -m pytest tests','npm run test -- one','pnpm test','yarn test','bun test','node --test','go test ./...','cargo test --all','dotnet test','mvn test','gradle test','./mvnw test -Dtest=Example','./gradlew test --tests Example','ant -f build.xml testclass -Dtest.entry=Example','java -cp build/classes:lib/junit.jar org.junit.runner.JUnitCore Example','cd /tmp/repo && defects4j test | tail -20','pytest tests -q 2>&1 | tail -20','pytest tests >result.log','pytest tests ; echo ok','printf tests','cargo check','npm run build','contest','echo pytestx'];
const commandRows=commands.map(command=>({command,recognized:sdk.isTestCommand(command),normalized:sdk.normalizeTestCommand(command)}));
const classifications=[
 ['pytest tests','10 passed',false],['pytest tests','1 failed',false],['pytest missing','collected 0 items',true],['pytest missing','collected 0 items / 2 skipped',true],
 ['pytest tests | grep FAIL','Command exited with code 1',true],['pytest tests','Command exited with code 1',true],['pytest tests','permission denied',true],['pytest tests','unrecognized arguments: --missing',true],
 ['pytest tests','/bin/bash: line 1: pytest: command not found',true],['pytest tests',"FAILED test_file.py::test_one - No such file or directory\n1 failed",true],
 ['pip install missing && pytest tests','No matching distribution found',true],['npm install && npm test','failed to fetch',true],
 ['defects4j test','Failing tests: 1',false],['pytest tests','test_b.py::test_b FAILED\nFAILED test_a.py::test_a\n',true],
 ['pytest tests','test_b.py::test_b PASSED\ntest_a.py::test_a PASSED\n',false],['git stash && pytest tests','2 passed',false],['pytest tests','Command timed out',true],['pytest tests','0 tests',false],
].map(([command,output,isError])=>({input:{command,output,isError},result:sdk.classifyCodingVerification({command,output,isError})}));
const assistant=(text,stopReason='stop',thinking='')=>({role:'assistant',stopReason,content:[...(thinking?[{type:'thinking',thinking}]:[]),{type:'text',text}]});
const continuations=[
 assistant('Let me fix both:'),assistant('Now, I need to run the tests.'),assistant('Implemented and verified. All tests pass.'),assistant('Should I run the tests?'),
 assistant('', 'length', 'reasoning'),assistant('', 'max_tokens','reasoning'),assistant('', 'stop','reasoning'),assistant('assistant','stop','reasoning'),assistant('<|im_start|>assistant<|im_end|>','stop','reasoning'),assistant('user'),assistant('x'.repeat(801)),assistant('Let me inspect the file:'),assistant('Done.'),
 {role:'assistant',content:[{type:'toolCall',name:'read'}],stopReason:'stop'},{role:'user',content:'continue'},assistant('Good. Next, update the code:'),assistant('Run tests now.'),assistant('What changed?'),
].map(message=>({message,result:sdk.assistantNeedsContinuationRecovery(message)}));
const tool=(id,name,args)=>({role:'assistant',content:[{type:'toolCall',id,name,arguments:args}],stopReason:'toolUse'});
const result=(id,name,text,isError=false)=>({role:'toolResult',toolCallId:id,toolName:name,content:[{type:'text',text}],isError});
const edit=(id='edit',name='edit')=>[tool(id,name,{path:'src/a.py'}),result(id,name,'patched')];
const test=(id='test',command='pytest tests -q',text='10 passed',error=false)=>[tool(id,'bash',{command}),result(id,'bash',text,error)];
const inputs=[
 {messages:[]},{messages:[...edit(),...test()]},{messages:[...test(),...edit()]},{messages:[...edit(),...test('bad','pytest tests','1 failed',true)]},
 {messages:[...edit(),...test('bad','pytest tests','1 failed',true),...test('narrow','pytest tests/one','1 passed')]},
 {messages:[...edit(),...test('bad','pytest tests','1 failed',true),...test('good','pytest tests','10 passed')]},
 {messages:[...edit(),...test('one'),...test('two')]},
 {messages:[...edit(),...test('none','pytest missing','collected 0 items',true)],untrackedPaths:['scratch.py']},
 {messages:[...edit(),...test()],modifiedTestPaths:['tests/test_a.py']},
 {messages:[...edit(),...test(),assistant('Let me fix it:')]},
 {messages:[...edit()],verificationAuthority:'typed-ledger'},
 {messages:[...edit()],verificationAuthority:'typed-ledger',untrackedPaths:['scratch']},
 ...['patch','multi_edit','append','NotebookEdit','notebook_edit'].map(name=>({messages:[...edit('edit',name),...test()]})),
];
const assessments=inputs.map(input=>({input,result:sdk.assessCodingCompletionGate(input)}));
const verify=(patchSha256,command,output,isError=false,baselineProbe)=>({kind:'verify',patchSha256,command,output,isError,...(baselineProbe===undefined?{}:{baselineProbe})});
const candidate=patchSha256=>({kind:'candidate',patchSha256});
const assess=patchSha256=>({kind:'assess',patchSha256});
const ledgerCases=[
 ['no-patch',[assess('')]],
 ['narrow-inherits',[candidate('a'),verify('a','pytest tests','FAILED test_a::bad\n1 failed',true),candidate('b'),verify('b','pytest tests/other','1 passed'),assess('b')]],
 ['broad-rerun',[candidate('a'),verify('a','pytest tests','1 failed',true),candidate('b'),verify('b','pytest tests','10 passed'),assess('b')]],
 ['exact-revert',[candidate('a'),verify('a','pytest tests','10 passed'),candidate('b'),verify('b','pytest tests','1 failed',true),candidate('a'),assess('a')]],
 ['baseline',[candidate('a'),verify('a','pytest tests','FAILED test_a::bad\n1 failed',true),verify('','git stash && pytest tests','FAILED test_a::bad\n1 failed',true,true),candidate('b'),verify('b','pytest tests/other','1 passed'),assess('b')]],
 ['baseline-prose',[candidate('a'),verify('a','pytest tests','FAILED test_a::bad\n1 failed',true),verify('a','git stash && pytest tests','FAILED test_a::bad\n1 failed',true,true),candidate('b'),verify('b','pytest tests/other','1 passed'),assess('b')]],
 ['passing-check',[candidate('a'),verify('a','pytest tests','FAILED test_a::bad\n1 failed',true),candidate('b'),verify('b','pytest all-tests','test_a::bad PASSED\n10 passed'),assess('b')]],
 ['duplicate-pass',[verify('a','pytest tests','1 passed'),verify('a','pytest tests','1 passed'),assess('a')]],
 ['failure-without-identities',[verify('a','pytest tests','1 failed',true),verify('','git stash && pytest tests','1 failed',true,true),verify('b','pytest other','1 passed'),assess('b')]],
];
const ledgers=ledgerCases.map(([id,steps])=>{
 const ledger=new sdk.CodingVerificationLedger();const outcomes=[];
 for(const step of steps){
  if(step.kind==='candidate')ledger.noteCandidate(step.patchSha256);
  else if(step.kind==='verify')outcomes.push({kind:'verify',result:ledger.noteVerification(step)});
  else outcomes.push({kind:'assess',result:ledger.assess(step.patchSha256)});
 }
 return {id,steps,outcomes};
});
const source='packages/coding-agent/src/core/coding-completion-gate.ts';
const captured=JSON.parse(JSON.stringify({version:1,source_commit:'e30f0b4b238ee5618928bcd17b4571eb046e7038',source_files:[{path:source,sha256:createHash('sha256').update(fs.readFileSync(path.join(base,'src/core/coding-completion-gate.ts'))).digest('hex')}],commands:commandRows,classifications,continuations,assessments,ledgers,model_actions:0,network_actions:0}));
const target=path.join(root,'fixtures/runtime/completion-oracle.json');
if(process.argv.includes('--capture')){assert(!fs.existsSync(target));fs.writeFileSync(target,JSON.stringify(captured,null,2)+'\n');}
else assert.deepEqual(captured,JSON.parse(fs.readFileSync(target,'utf8')));
console.log(JSON.stringify({status:'passed',commands:commandRows.length,classifications:classifications.length,continuations:continuations.length,assessments:assessments.length,ledgers:ledgers.length,model_actions:0,network_actions:0}));
