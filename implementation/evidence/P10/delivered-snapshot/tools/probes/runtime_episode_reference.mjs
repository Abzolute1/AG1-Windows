// Unchanged SDK episode tool; disposable journal/artifact ports, no model calls.
import fs from 'node:fs';import os from 'node:os';import path from 'node:path';import assert from 'node:assert/strict';import {createHash} from 'node:crypto';import {fileURLToPath,pathToFileURL} from 'node:url';
await import('../offline_guard.mjs');
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../..');
const base=path.join(root,'reference/ag1-executable/packages/coding-agent');
const {createEpisodeToolDefinition}=await import(pathToFileURL(path.join(base,'dist/core/tools/episode.js')));
const fixture=JSON.parse(fs.readFileSync(path.join(root,'fixtures/context/episode-inputs.json'),'utf8'));
const now='2026-09-06T00:00:00.000Z',cwd=fs.mkdtempSync(path.join(os.tmpdir(),'ag1-episode-oracle-'));
const open={type:'custom',customType:'context_episode',id:'open',parentId:null,timestamp:now,data:{version:1,event:'begin',id:'ep_000000000001',goal:'Repair compiler startup',title:'Compiler diagnosis',createdAt:now}};
const task={type:'custom',customType:'task_state',id:'task',parentId:'open',timestamp:now,data:{version:1,status:'active',objective:'Repair compiler startup',constraints:[],openQuestions:[],entries:[],updatedAt:now}};
const check={type:'message',id:'check-result',parentId:'task',timestamp:now,message:{role:'toolResult',toolName:'check',toolCallId:'check-call',isError:false,details:{passed:true},content:[{type:'text',text:'Passed'}]}};
const close={action:'close',outcome:'completed',synopsis:' Compiler startup\n diagnosed '};
const cases=[
 {id:'begin',branch:[],args:{action:'begin',goal:'  Repair compiler\n startup  ',title:' Diagnosis '}},
 {id:'begin-already-open',branch:[open],args:{action:'begin',goal:'Another goal'}},
 {id:'empty-goal',branch:[],args:{action:'begin',goal:'  '}},
 {id:'empty-list',branch:[],args:{action:'list'}},
 {id:'open-list',branch:[open],args:{action:'list'}},
 {id:'open-show',branch:[open,task],args:{action:'show',id:open.data.id}},
 {id:'unknown-show',branch:[open],args:{action:'show',id:'ep_111111111111'}},
 {id:'closed-list',branch:fixture.entries,args:{action:'list',limit:2}},
 {id:'closed-show',branch:fixture.entries,args:{action:'show',id:fixture.records[1].id}},
 ...fixture.queries.map((query,i)=>({id:`search-${i}`,branch:fixture.entries,args:{action:'search',query}})),
 {id:'close-no-task',branch:[open],args:close},
 {id:'close-unsupported-task',branch:[open,task],args:close},
 {id:'close-passing-check',branch:[open,task,check],args:{...close,verificationSources:['tool-call:check-call']}},
 {id:'close-old-check',branch:[{...check,parentId:null},{...open,parentId:check.id},task],args:{...close,verificationSources:['tool-call:check-call']}},
 {id:'close-dirty-after-check',branch:[open,task,check,{type:'message',id:'write-result',parentId:check.id,timestamp:now,message:{role:'toolResult',toolName:'write',toolCallId:'write-call',isError:false,content:[]}}],args:{...close,verificationSources:['tool-call:check-call']}},
 {id:'close-artifact',branch:[open,task],args:{...close,verificationSources:[fixture.artifacts[0].id]}},
 {id:'close-missing-artifact',branch:[open,task],args:{...close,verificationSources:['art_ffffffffffffffffffffffffffffffff']}},
 {id:'close-blocker',branch:[open,{...task,data:{...task.data,blockerId:'blocked'}}],args:{...close,verificationSources:[fixture.artifacts[0].id]}},
 {id:'close-partial',branch:[open,task],args:{...close,outcome:'partial'}},
 {id:'close-unsourced-claim',branch:[open],args:{...close,claims:[{kind:'FACT',statement:'Unsupported fact',sources:[]}]}},
];
const OriginalDate=globalThis.Date,rows=[];
try {
 globalThis.Date=class extends OriginalDate{constructor(...args){super(...(args.length?args:[now]));}static now(){return new OriginalDate(now).getTime();}};
 for(const test of cases) {
  const branch=structuredClone(test.branch),records=[],artifacts=structuredClone(fixture.artifacts);
  const store={get(id){const found=artifacts.find(a=>a.id===id);if(!found)throw Error('missing');return found;},putText(text,options){const artifact={id:'art_eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee',bytes:Buffer.byteLength(text),kind:options.kind};artifacts.push(artifact);return artifact;}};
  const manager={getBranch:()=>branch,appendCustomEntryAsync:async(customType,data)=>{records.push({type:'custom',customType,data});return 'published';}};
  const tool=createEpisodeToolDefinition({cwd,artifactStore:store,sessionManager:manager,sourceRevisionValidator:{validate:source=>({source,status:fixture.freshness[source]??'invalid'})}});
  let output,error;try{output=await tool.execute('episode-call',test.args);}catch{error=true;}
  let row={...test,...(error?{error}:{output}),records};
  if(output&&test.args.action==='begin')row=JSON.parse(JSON.stringify(row).replaceAll(output.details.id,'ep_000000000001'));
  rows.push(row);
 }
}finally{globalThis.Date=OriginalDate;fs.rmSync(cwd,{recursive:true,force:true});}
const result={version:1,source_commit:'e30f0b4b238ee5618928bcd17b4571eb046e7038',source_files:['tools/episode','episode-memory','episode-retrieval','task-state'].map(name=>({path:`packages/coding-agent/src/core/${name}.ts`,sha256:createHash('sha256').update(fs.readFileSync(path.join(base,`src/core/${name}.ts`))).digest('hex')})),now,artifacts:fixture.artifacts,freshness:fixture.freshness,rows,model_actions:0,network_actions:0};
const target=path.join(root,'fixtures/runtime/episode-tool-oracle.json');
if(process.argv.includes('--capture')){assert(!fs.existsSync(target));fs.writeFileSync(target,JSON.stringify(result,null,2)+'\n');}else assert.deepEqual(JSON.parse(JSON.stringify(result)),JSON.parse(fs.readFileSync(target,'utf8')));
console.log(JSON.stringify({status:'passed',episode_cases:rows.length,model_actions:0,network_actions:0}));
