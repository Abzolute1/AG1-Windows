import fs from 'node:fs';import path from 'node:path';import os from 'node:os';import assert from 'node:assert/strict';import {fileURLToPath,pathToFileURL} from 'node:url';import {createHash} from 'node:crypto';
await import('../offline_guard.mjs');
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../..');
const {ArtifactStore}=await import(pathToFileURL(path.join(root,'reference/ag1-executable/packages/coding-agent/dist/core/artifact-store.js')));
const {compileContextWorkingSet}=await import(pathToFileURL(path.join(root,'reference/ag1-executable/packages/coding-agent/dist/core/context-compiler.js')));
function canonical(v){if(Array.isArray(v))return v.map(canonical);if(v&&typeof v==='object')return Object.fromEntries(Object.keys(v).sort().map(k=>[k,canonical(v[k])]));return v;}
const recipes=JSON.parse(fs.readFileSync(path.join(root,'fixtures/context/compiler-inputs.json'),'utf8'));const groups=[];
for(const recipe of recipes.cases){const temporary=fs.mkdtempSync(path.join(os.tmpdir(),'ag1-context-reference-'));
 try{
  const store=new ArtifactStore(temporary,'fixture',{features:{typedEvidenceCursors:true,evidenceLeases:true,segmentedArtifactCatalog:true},now:()=>1000});
  const ids=new Map(),records=[];const original=store.putText.bind(store);
  store.putText=(text,options)=>{const r=original(text,options);ids.set(r.id,`art_${(ids.size+1).toString(16).padStart(32,'0')}`);records.push({record:r,text});return r;};
  const user=text=>({role:'user',content:text,timestamp:1});const messages=[user('Implement the repository task')];let seed;
  if(recipe.seed){seed=store.putText('CURRENT_CONTRACT: preserve exact evidence\n'+'seed '.repeat(1000),{kind:'test-output',producer:{type:'tool',name:'check',callId:'seed'},salientLines:['CURRENT_CONTRACT: preserve exact evidence']});if(recipe.seed==='lease')store.leaseEvidence(seed.id,'required proof',{ttlMs:60000});}
  for(let i=0;i<recipe.rounds;i++){
   const tool=recipe.tool??'bash',thinking=(i>=recipe.rounds-3?recipe.lastThinkingBytes:undefined)??recipe.thinkingBytes;
   const args=recipe.payloadBytes?{content:'x'.repeat(recipe.payloadBytes),path:'src/a.rs'}:tool==='read'?{path:'src/a.rs'}:{command:'check'};
   messages.push({role:'assistant',content:[{type:'thinking',thinking:'r'.repeat(thinking)},{type:'toolCall',id:`c${i}`,name:tool,arguments:args}],api:'openai-completions',provider:'local',model:'fixture',usage:{input:0,output:0,cacheRead:0,cacheWrite:0,total:0},stopReason:'toolUse',timestamp:1});
   const raw=`round ${i}\nCURRENT_CONTRACT: keep evidence ${i}\n${(recipe.unicode?'β🌙':'x').repeat(recipe.textBytes)}\nno actionable evidence follows`;
   const content=Array.from({length:recipe.blocks??1},()=>({type:'text',text:raw}));let result={role:'toolResult',toolCallId:`c${i}`,toolName:tool,content,isError:false,timestamp:1,details:{fixture:true}};
   if(i===0&&seed){result=recipe.seed==='read'?{...result,toolName:'artifact',details:{action:'read',id:seed.id}}:{...result,content:[{type:'text',text:`artifact: ${seed.id}\nfull_result: exact artifact, paged by UTF-8 byte offset`}],details:{id:seed.id}};}
   messages.push(result);
  }
  if(recipe.finalUser)messages.push(user('Continue using the exact evidence above'));
  const input=canonical(messages),options={contextWindow:recipe.window,...(recipe.recent?{recentAssistantTurns:recipe.recent}:{}),...(recipe.metadataBudget?{maxResidentArtifactMetadataTokens:recipe.metadataBudget}:{}),...(recipe.fixed?{fixedTokens:recipe.fixed}:{})};
  const cold=compileContextWorkingSet(input,{...options,artifactStore:store});const coldCount=records.length;const warm=compileContextWorkingSet(cold.messages,{...options,artifactStore:store});
  function normalize(value){return JSON.parse(JSON.stringify(value,(key,v)=>key==='durationMs'?undefined:typeof v==='string'?v.replace(/art_[0-9a-f]{32}/g,id=>{assert(ids.has(id));return ids.get(id)}):v));}
  const artifacts=records.map(({record:r,text})=>{let offset=0,recovered='';while(offset<r.bytes){const page=store.read(r.id,offset,32768);recovered+=page.text;offset=page.offset+Buffer.byteLength(page.text);}assert.equal(recovered,text);assert.equal(createHash('sha256').update(text).digest('hex'),r.sha256);return normalize({id:r.id,kind:r.kind,bytes:r.bytes,sha256:r.sha256,lineCount:r.lineCount,...(r.salientLines?{salientLines:r.salientLines}:{}),producer:r.producer});});
  groups.push({id:recipe.id,inputSha256:createHash('sha256').update(JSON.stringify(normalize(input))).digest('hex'),options,cold:normalize(cold),warm:normalize(warm),coldArtifacts:coldCount,artifacts});
 }finally{fs.rmSync(temporary,{recursive:true,force:true});}}
const result={version:1,source_commit:'e30f0b4b238ee5618928bcd17b4571eb046e7038',model_actions:0,network_actions:0,normalization:['Recipe input object keys are sorted before either implementation receives them. Only occurrence IDs and durationMs are normalized in captured outputs; selected evidence text/order, hashes and statistics are compared unchanged.'],groups};
const target=path.join(root,'fixtures/context/compiler-oracle.json');if(process.argv.includes('--capture')){assert(!fs.existsSync(target));fs.writeFileSync(target,JSON.stringify(result,null,2)+'\n');}else assert.deepEqual(result,JSON.parse(fs.readFileSync(target,'utf8')));
console.log(JSON.stringify({status:'passed',compiler_cases:groups.length,model_actions:0,network_actions:0}));
