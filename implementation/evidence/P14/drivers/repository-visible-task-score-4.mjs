// Adapt the unchanged original residency criterion to actual outgoing messages.
// Durable native history and model-visible compiled history are separate objects.
import fs from 'node:fs';import path from 'node:path';import crypto from 'node:crypto';import assert from 'node:assert/strict';import {pathToFileURL} from 'node:url';
const root=process.cwd(),[pairArg,adapter,outArg]=process.argv.slice(2),pair=path.resolve(pairArg),out=path.resolve(outArg);
assert(['ts','rust'].includes(adapter));assert(pair.startsWith(path.join(root,'.local/p14-repository/')));assert(out.startsWith(path.join(root,'.local/p14-repository-visible-score-4/')));assert(!fs.existsSync(out));
const base=path.join(pair,adapter),scorer=path.join(root,'.local/p14-repository-working-set-preparation/original-repository-runner.mjs'),originalPath=path.join(base,adapter==='ts'?'typescript-result.json':'native-result.json'),original=JSON.parse(fs.readFileSync(originalPath)),benchmarkCase=JSON.parse(fs.readFileSync(path.join(base,'case.json')));
assert.equal(original.fixture,false);assert.equal(original.unchanged_original_verifiers,true);
const suites=JSON.parse(fs.readFileSync(path.join(root,'.local/p14-repository-working-set-preparation/suites.json'))),suite=suites.find(s=>s.suite.cases.some(c=>c.id===benchmarkCase.id)).suite;
const {createBenchmark}=await import(pathToFileURL(scorer));const {isExternalizedToolResultText}=await import(pathToFileURL(path.join(root,'reference/ag1-executable/packages/coding-agent/dist/core/tool-result-artifacts.js')));const b=createBenchmark({isExternalizedToolResultText});
const allNames=fs.readdirSync(base).filter(n=>(adapter==='ts'?/^transport-\d+\.json$/:/^request-\d+\.json$/).test(n)).sort();assert(allNames.length);
const boundaryPath=path.join(base,'task-complete-before-debrief.json');
const boundary=fs.existsSync(boundaryPath)?JSON.parse(fs.readFileSync(boundaryPath)):null;
const taskRequests=boundary?(adapter==='ts'?boundary.transports:boundary.requests):allNames.length;
assert(Number.isSafeInteger(taskRequests)&&taskRequests>0&&taskRequests<=allNames.length);
const names=allNames.slice(0,taskRequests);
for(const [index,name] of names.entries())assert.equal(Number(name.match(/-(\d+)\.json$/)[1]),index+1);
const ref=p=>({path:path.relative(root,p),sha256:crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex')});
const noArtifactQueries={async grep(){throw new Error('Residency observation cannot retrieve new evidence');}};
const observations=[];
for(const name of names){
 const body=JSON.parse(fs.readFileSync(path.join(base,name))).body;assert(Array.isArray(body.messages));const toolMessages=body.messages.filter(m=>m.role==='tool').map(m=>{assert.equal(typeof m.tool_call_id,'string');assert.equal(typeof m.content,'string');return {role:'toolResult',toolCallId:m.tool_call_id,content:[{type:'text',text:m.content}]};});
 const telemetry=await b.inspectRepositoryArtifactRehydrations(toolMessages,noArtifactQueries,benchmarkCase.history.rehydrationQuery);assert.equal(telemetry.artifactRehydrations,0);assert.equal(telemetry.evidenceRehydrations,0);
 observations.push({request:ref(path.join(base,name)),residentEvidenceReferences:telemetry.residentEvidenceReferences,visibleTools:toolMessages.length,matchingTools:toolMessages.filter(m=>isExternalizedToolResultText(m.content[0].text)&&benchmarkCase.history.rehydrationQuery?.trim()&&m.content[0].text.includes(benchmarkCase.history.rehydrationQuery)).map(m=>({toolCallId:m.toolCallId,visibleText:m.content[0].text,visibleTextSha256:crypto.createHash('sha256').update(m.content[0].text).digest('hex')}))});
}
const count=observations.at(-1).residentEvidenceReferences;
const result={...original.result,residentEvidenceReferences:count},evaluation=b.evaluateVariant({...suite,cases:[benchmarkCase]},'candidate',[result]);
// No original check or threshold changes: only the observed residency input changes.
const unchanged={...result};delete unchanged.residentEvidenceReferences;const prior={...original.result};delete prior.residentEvidenceReferences;assert.deepEqual(unchanged,prior);
if(adapter==='ts'){assert.equal(count,original.result.residentEvidenceReferences,'TS compiled final session must agree with actual final outgoing projection');assert.deepEqual(evaluation,original.evaluation);}
// Negative control: matching text in a user message or unmarked raw journal text
// is not residency under the original criterion. Erasing the exact marker from
// the visible tool projection removes its credit, even while source/journal retain it.
const needle=benchmarkCase.history.rehydrationQuery;let controls={executed:false};
if(needle?.trim()&&count>0){
 const text=observations.at(-1).matchingTools[0].visibleText;
 for(const control of [{role:'user',content:[{type:'text',text}]},{role:'toolResult',content:[{type:'text',text:text.replaceAll(needle,'REDACTED_REVIEW_NEGATIVE_CONTROL')}]},{role:'toolResult',content:[{type:'text',text:needle}]}]){
  const t=await b.inspectRepositoryArtifactRehydrations([control],noArtifactQueries,needle);assert.equal(t.residentEvidenceReferences,0);
 }
 const negative=b.evaluateVariant({...suite,cases:[benchmarkCase]},'candidate',[{...result,residentEvidenceReferences:0}]);
 if(benchmarkCase.candidateMustHaveExactEvidence&&result.evidenceRehydrations===0)assert.equal(negative.passed,false);
 controls={executed:true,userTextDoesNotCount:true,removedVisibleExactLineDoesNotCount:true,unmarkedStoredTextDoesNotCount:true,originalGateStillRejectsMissingRequiredEvidence:benchmarkCase.candidateMustHaveExactEvidence&&result.evidenceRehydrations===0?!negative.passed:null};
}
fs.mkdirSync(out,{mode:0o700});const report={taskBoundary:boundary?ref(boundaryPath):null,taskRequestCount:taskRequests,excludedDebriefRequests:allNames.length-taskRequests,status:evaluation.passed&&original.unchanged_original_verifiers?'passed':'failed',fixture:false,release_acceptance:false,adapter,case:benchmarkCase.id,result,evaluation,observations,negative_controls:controls,original_result:ref(originalPath),original_scorer:ref(scorer),original_suite:ref(path.join(root,'.local/p14-repository-working-set-preparation/suites.json')),adapter_source:ref(path.resolve(process.argv[1])),scope:'Unchanged original scorer/criteria; residency input measured on actual final outgoing compiled tool messages. Original verifier, mutations, tokens, tool/retrieval counts and failures preserved. No new model request or evidence retrieval. Intermediate requests retained to reveal whether exact evidence remained visible.',raw_status:original.status,raw_residentEvidenceReferences:original.result.residentEvidenceReferences};
fs.writeFileSync(path.join(out,'result.json'),JSON.stringify(report,null,2)+'\n',{flag:'wx'});console.log(adapter,benchmarkCase.id,'raw',original.status,'model-visible',report.status,'resident',original.result.residentEvidenceReferences,'->',count);
