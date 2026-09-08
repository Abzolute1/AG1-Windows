// Reuse the pinned benchmark's exact prompt assembly and scoring; no model control.
import fs from 'node:fs';
import path from 'node:path';
import assert from 'node:assert/strict';
import crypto from 'node:crypto';
import {pathToFileURL} from 'node:url';
const root=path.resolve(import.meta.dirname,'..');
const reference=path.join(root,'reference/ag1-executable');
const source=path.join(reference,'packages/coding-agent/dist/core/context-intelligence-benchmark.js');
const {runContextIntelligenceBenchmark}=await import(pathToFileURL(source));
const suitePath=path.join(reference,'packages/coding-agent/test/fixtures/context-intelligence/provenance-parity.json');
const suite=JSON.parse(fs.readFileSync(suitePath));
const [mode,directory]=process.argv.slice(2);
const base=path.resolve(directory);
assert(base.startsWith(path.join(root,'.local/p14-quality/')));
const hash=p=>crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
const cases=[];
await runContextIntelligenceBenchmark(suite,async request=>{
  const fresh=`BENCHMARK_INVOCATION trial=1 case=${request.caseId} variant=${request.variant}; this label carries no engineering evidence.`;
  cases.push({id:`${request.caseId}:${request.variant}`,context:{systemPrompt:`${fresh}\n${request.systemPrompt}`,
    messages:[{role:'user',content:request.prompt,timestamp:0}]},options:{maxTokens:request.maxTokens}});
  return {text:''}; // Capture only; no scoring result from this invocation is accepted.
},{maxOutputTokens:2048,maximumRegression:0,orderOffset:0});
assert.equal(cases.length,suite.cases.length*2);
if(mode==='capture'){
  fs.mkdirSync(base,{recursive:true,mode:0o700});
  fs.writeFileSync(path.join(base,'cases.json'),JSON.stringify(cases,null,2),{flag:'wx',mode:0o600});
  fs.writeFileSync(path.join(base,'provenance.json'),JSON.stringify({source,source_sha256:hash(source),suite:suitePath,suite_sha256:hash(suitePath),
    case_count:cases.length,scope:'Pinned benchmark prompt/score logic unchanged; both language adapters receive both fixture variants. Variants are not programming languages.',model_actions:0}),{flag:'wx',mode:0o600});
  console.log(`Captured ${cases.length} unchanged context-intelligence prompts`);
}else if(mode==='score'){
  const execution=JSON.parse(fs.readFileSync(path.join(base,'matrix/execution.json')));
  assert.equal(execution.fixture,false);
  assert.equal(execution.status,'executed');assert.equal(execution.runtime_settled,true);
  assert.deepEqual(JSON.parse(fs.readFileSync(path.join(base,'cases.json'))),cases);
  const rows=fs.readFileSync(path.join(base,'matrix/rows.jsonl'),'utf8').trim().split('\n').map(JSON.parse);
  assert.equal(rows.length,execution.expected_rows);
  const models=[...new Set(rows.map(r=>r.model))];assert.equal(models.length,3);
  for(const model of models)for(const item of cases){
    const pair=rows.filter(r=>r.model===model&&r.case===item.id);
    assert.equal(pair.length,2);assert.deepEqual(pair.map(r=>r.runner),['typescript','rust']);
    assert.deepEqual(pair[0].answer.payload,pair[1].answer.payload,'Language adapters must receive identical assembled requests');
  }
  const results=[];
  for(const model of models)for(const runner of ['typescript','rust']){
    const selected=rows.filter(r=>r.model===model&&r.runner===runner);
    assert.equal(selected.length,cases.length);
    const result=await runContextIntelligenceBenchmark(suite,async request=>{
      const row=selected.find(r=>r.case===`${request.caseId}:${request.variant}`);assert(row);
      assert.equal(row.answer.settled,true);
      const content=row.answer.message.content??[];
      return {text:content.filter(b=>b.type==='text').map(b=>b.text).join(''),
        inputTokens:row.answer.message.usage?.input,outputTokens:row.answer.message.usage?.output};
    },{maxOutputTokens:2048,maximumRegression:0,orderOffset:0});
    results.push({model,runner,result});
  }
  const report={status:'scored',fixture:false,scope:'Frozen-prompt context-intelligence replay. This evaluates output correctness, evidence recall and stale/unsupported claims; it does not establish full-session harness coherence or superiority.',
    cases:cases.length,rows:rows.length,results,
    all_tasks_succeeded:results.every(r=>r.result.results.every(v=>v.observation.taskSucceeded)),
    unchanged_verifier:{path:source,sha256:hash(source)}};
  fs.writeFileSync(path.join(base,'quality.json'),JSON.stringify(report,null,2),{flag:'wx',mode:0o600});
  console.log(JSON.stringify(results.map(r=>({model:r.model,runner:r.runner,tasks:r.result.results.filter(v=>v.observation.taskSucceeded).length,total:r.result.results.length,
    staleClaims:r.result.results.reduce((s,v)=>s+v.observation.staleAuthoritativeClaims,0),unsupportedClaims:r.result.results.reduce((s,v)=>s+v.observation.unsupportedAuthoritativeClaims,0)})),null,2));
}else throw Error('Use capture or score');
