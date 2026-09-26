// Normalize successful owner-issued cursor reads for the unchanged ID-based scorer.
// Version 1's separate compiled-residency measurement remains an immutable input.
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import assert from 'node:assert/strict';
import {pathToFileURL} from 'node:url';

const root=process.cwd(),[pairArg,adapter,outArg,firstArg]=process.argv.slice(2);
const pair=path.resolve(pairArg),out=path.resolve(outArg),base=path.join(pair,adapter);
assert(['ts','rust'].includes(adapter));
assert(pair.startsWith(path.join(root,'.local/p14-repository/')));
assert(out.startsWith(path.join(root,'.local/p14-repository-visible-score-3/'))&&!fs.existsSync(out));
const read=p=>JSON.parse(fs.readFileSync(p));
const hash=b=>crypto.createHash('sha256').update(b).digest('hex');
const ref=p=>({path:path.relative(root,p),sha256:hash(fs.readFileSync(p))});
assert(firstArg, 'Explicit version 1 result path is required');
const firstPath=path.resolve(firstArg);
assert(firstPath.startsWith(path.join(root,'.local/p14-repository-visible-score-1/')));
assert(!fs.lstatSync(firstPath).isSymbolicLink());
const first=read(firstPath),rawPath=path.join(base,adapter==='ts'?'typescript-result.json':'native-result.json'),raw=read(rawPath);
assert.deepEqual(first.original_result,ref(rawPath));
assert.equal(raw.fixture,false);assert.equal(raw.unchanged_original_verifiers,true);
const benchmarkCase=read(path.join(base,'case.json')),needle=benchmarkCase.history.rehydrationQuery;
const scorer=path.join(root,'.local/p14-repository-working-set-preparation/original-repository-runner.mjs');
assert.equal(hash(fs.readFileSync(scorer)),first.original_scorer.sha256);
const suites=read(path.join(root,'.local/p14-repository-working-set-preparation/suites.json'));
const suite=suites.find(s=>s.suite.cases.some(c=>c.id===benchmarkCase.id)).suite;
const {createBenchmark}=await import(pathToFileURL(scorer));
const {isExternalizedToolResultText}=await import(pathToFileURL(path.join(root,'reference/ag1-executable/packages/coding-agent/dist/core/tool-result-artifacts.js')));
const benchmark=createBenchmark({isExternalizedToolResultText});
const seed=read(path.join(base,'seed.json')),journal=path.resolve(seed.journal);
assert(journal.startsWith(pair+path.sep));assert(!fs.lstatSync(journal).isSymbolicLink());
const sourceBlocks=new Map();
for(const [index,line] of fs.readFileSync(journal,'utf8').trimEnd().split('\n').entries()){
  const record=JSON.parse(line),m=record.message;
  if(m?.role!=='toolResult')continue;
  for(const [blockIndex,block] of m.content.entries())if(block.type==='text'){
    const bytes=Buffer.from(block.text);
    sourceBlocks.set(hash(bytes),{bytes,index,blockIndex});
  }
}
const names=fs.readdirSync(base).filter(n=>(adapter==='ts'?/^transport-\d+\.json$/:/^request-\d+\.json$/).test(n)).sort();
assert(names.length);
const requestPath=path.join(base,names.at(-1)),messages=read(requestPath).body.messages;
assert(Array.isArray(messages));
const calls=new Map(),issued=new Map(),identities=new Map(),proofs=[],controlResults=[];
const idPattern=/^art_[0-9a-f]{32}$/;
function headerOf(m){
  if(m.role!=='tool'||typeof m.content!=='string')return null;
  const newline=m.content.indexOf('\n');
  try{return {header:JSON.parse(newline<0?m.content:m.content.slice(0,newline)),payload:newline<0?'':m.content.slice(newline+1)};}
  catch{return null;}
}
function trustedContract(x){return x?.authenticated===true&&x?.sessionScoped===true&&x?.sourceImmutable===true;}
function prove(call,message){
  if(call?.function?.name!=='artifact')return null;
  const args=JSON.parse(call.function.arguments);
  if(args.action!=='read'||typeof args.id==='string'||typeof args.cursor!=='string')return null;
  if(message.role!=='tool'||message.tool_call_id!==call.id)return null;
  const parsed=headerOf(message),page=parsed?.header?.artifactPage,origin=issued.get(args.cursor);
  if(!origin||!page||page.id!==origin.id||!trustedContract(page.cursorContract))return null;
  const identity=identities.get(page.id),source=identity&&sourceBlocks.get(identity.sha256),range=page.range;
  if(!source||!range||!['startByte','endByte','totalBytes'].every(k=>Number.isSafeInteger(range[k])))return null;
  if(range.startByte<0||range.endByte<=range.startByte||range.endByte>range.totalBytes||range.totalBytes!==source.bytes.length)return null;
  const payload=Buffer.from(parsed.payload);
  if(!payload.equals(source.bytes.subarray(range.startByte,range.endByte)))return null;
  if(!needle?.trim()||!parsed.payload.includes(needle))return null;
  return {artifactId:page.id,cursorSha256:hash(args.cursor),issuedAtMessage:origin.index,
    callId:call.id,sourceSha256:identity.sha256,sourceJournal:ref(journal),
    sourceJournalRecord:source.index,sourceBlock:source.blockIndex,
    range,payloadSha256:hash(payload),payloadBytes:payload.length,exactQueryVisible:true};
}
function indexCursors(node,inheritedId,index){
  if(!node||typeof node!=='object')return;
  const id=idPattern.test(node.id)?node.id:inheritedId;
  for(const name of ['readCall','nextCall']){
    const next=node[name];
    if(id&&next?.action==='read'&&typeof next.cursor==='string'){
      const old=issued.get(next.cursor);assert(!old||old.id===id);
      issued.set(next.cursor,{id,index});
    }
  }
  for(const value of Object.values(node))if(value&&typeof value==='object')indexCursors(value,id,index);
}
for(const [index,m] of messages.entries()){
  if(m.role==='assistant')for(const call of m.tool_calls??[]){assert(!calls.has(call.id));calls.set(call.id,call);}
  if(m.role!=='tool')continue;
  if(isExternalizedToolResultText(m.content)){
    const id=m.content.match(/^artifact: (art_[0-9a-f]{32})$/m)?.[1];
    const sha256=m.content.match(/^sha256: ([0-9a-f]{64})$/m)?.[1];
    if(id&&sha256&&sourceBlocks.has(sha256))identities.set(id,{sha256,index});
  }
  const call=calls.get(m.tool_call_id),proof=prove(call,m);
  if(proof){
    assert(proof.issuedAtMessage<index);
    const args=JSON.parse(call.function.arguments);
    const variants=[
      ['unissued cursor', {...call,function:{...call.function,arguments:JSON.stringify({...args,cursor:args.cursor+'x'})}},m],
      ['user text is not tool evidence',call,{...m,role:'user'}],
      ['wrong result identity',call,{...m,content:m.content.replace(proof.artifactId,'art_'+'0'.repeat(32))}],
      ['altered returned exact bytes',call,{...m,content:m.content.replaceAll(needle,'REDACTED_CURSOR_NEGATIVE_CONTROL')}],
      ['wrong byte range',call,{...m,content:m.content.replace(`"startByte":${proof.range.startByte}`,`"startByte":${proof.range.startByte+1}`)}],
    ];
    for(const [label,candidate,result] of variants){assert.equal(prove(candidate,result),null,label);controlResults.push({callId:call.id,label,rejected:true});}
    // The old criterion counts an ID-bearing read of an artifact containing the
    // query. This adapter has the stronger proof that the returned bytes contain it.
    const t=await benchmark.inspectRepositoryArtifactRehydrations([
      {role:'assistant',content:[{type:'toolCall',name:'artifact',arguments:{action:'read',id:proof.artifactId}}]}
    ],{async grep(id,query){assert.equal(id,proof.artifactId);assert.equal(query,needle);return {matches:[{matchedText:needle}]};}},needle);
    assert.equal(t.artifactRehydrations,1);assert.equal(t.evidenceRehydrations,1);
    proofs.push({...proof,originalCriterionCredit:t.evidenceRehydrations});
  }
  if(call?.function?.name==='artifact'){
    const parsed=headerOf(m);
    for(const key of ['artifactSearch','artifactGrep','artifactPage']){
      const header=parsed?.header?.[key];
      if(header&&trustedContract(header.cursorContract))indexCursors(header,undefined,index);
    }
  }
}
const result={...first.result,evidenceRehydrations:first.result.evidenceRehydrations+proofs.length};
assert(result.evidenceRehydrations<=result.artifactRehydrations);
const unchanged={...result};delete unchanged.evidenceRehydrations;
const before={...first.result};delete before.evidenceRehydrations;assert.deepEqual(unchanged,before);
const evaluation=benchmark.evaluateVariant({...suite,cases:[benchmarkCase]},'candidate',[result]);
if(!proofs.length)assert.deepEqual(evaluation,first.evaluation);
const report={status:evaluation.passed?'passed':'failed',fixture:false,release_acceptance:false,
  adapter,case:benchmarkCase.id,result,evaluation,rawStatus:raw.status,visibleOnlyStatus:first.status,
  cursorReadProofs:proofs,negativeControls:controlResults,originalReport:ref(rawPath),
  compiledVisibilityReport:ref(firstPath),outgoingRequest:ref(requestPath),originalScorer:ref(scorer),
  adapterSource:ref(path.resolve(process.argv[1])),
  scope:'Unchanged original verifier and evidence criteria. Only missing ID attribution for a successful cursor read is normalized, after matching a previously owner-issued cursor, returned artifact identity, original immutable source hash and exact returned byte range containing the required query. Original retrieval/tool/token counts stay unchanged. No search-action reclassification or new model request. Raw reports retained.'};
fs.mkdirSync(out,{mode:0o700});fs.writeFileSync(path.join(out,'result.json'),JSON.stringify(report,null,2)+'\n',{flag:'wx'});
console.log(adapter,benchmarkCase.id,'raw',raw.status,'corrected',report.status,'verified cursor reads',proofs.length);
