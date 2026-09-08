// Score the selected diagnostic answer with the unchanged original verifier.
// Other fixture answers come from the retained run and are excluded from reporting.
import fs from 'node:fs';
import path from 'node:path';
import assert from 'node:assert/strict';
import {pathToFileURL} from 'node:url';
const root=path.resolve(import.meta.dirname,'..');
const [output,historical]=process.argv.slice(2).map(p=>path.resolve(p));
assert(output.startsWith(path.join(root,'.local/p14-quality/')));
assert(historical.startsWith(path.join(root,'.local/p14-quality/')));
const reference=path.join(root,'reference/ag1-executable');
const {runContextIntelligenceBenchmark}=await import(pathToFileURL(path.join(reference,'packages/coding-agent/dist/core/context-intelligence-benchmark.js')));
const suite=JSON.parse(fs.readFileSync(path.join(reference,'packages/coding-agent/test/fixtures/context-intelligence/provenance-parity.json')));
const rows=p=>fs.readFileSync(path.join(p,'matrix/rows.jsonl'),'utf8').trim().split('\n').map(JSON.parse);
const observed=rows(output),old=rows(historical);
assert.equal(observed.length,8);
const scores=[];
for(let trial=0;trial<4;trial++){
  const pair=observed.filter(r=>r.trial===trial);
  assert.equal(pair.length,2);
  assert.deepEqual(pair.map(r=>r.runner),trial%2?['rust','typescript']:['typescript','rust']);
  assert.deepEqual(pair[0].answer.payload,pair[1].answer.payload);
  for(const row of pair){
    const prior=old.find(r=>r.runner===row.runner&&r.case===row.case&&r.model===row.model);
    assert(prior);assert.deepEqual(row.answer.payload,prior.answer.payload);
    const result=await runContextIntelligenceBenchmark(suite,async request=>{
      const id=`${request.caseId}:${request.variant}`;
      const selected=id===row.case?row:old.find(r=>r.runner===row.runner&&r.case===id&&r.model===row.model);
      assert(selected&&selected.answer.settled);
      return {text:selected.answer.message.content.filter(b=>b.type==='text').map(b=>b.text).join('')};
    },{maxOutputTokens:2048,maximumRegression:0,orderOffset:0});
    const selected=result.results.find(r=>`${r.caseId}:${r.variant}`===row.case);assert(selected);
    scores.push({trial,runner:row.runner,model:row.model,case:row.case,result:selected});
  }
}
fs.writeFileSync(path.join(output,'scores.json'),JSON.stringify(scores,null,2),{flag:'wx',mode:0o600});
console.log(JSON.stringify(scores.map(r=>({trial:r.trial,runner:r.runner,observation:r.result.observation})),null,2));
