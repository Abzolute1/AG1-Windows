import fs from 'node:fs';
import path from 'node:path';
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {fileURLToPath,pathToFileURL} from 'node:url';
await import('../offline_guard.mjs');
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../..');
const base=path.join(root,'reference/ag1-executable');
const {collectEntriesForBranchSummary,prepareBranchEntries}=await import(pathToFileURL(path.join(base,'packages/coding-agent/dist/core/compaction/branch-summarization.js')));
const now='2026-09-06T00:00:00.000Z';
const message=(id,parentId,role,content)=>({type:'message',id,parentId,timestamp:now,message:{role,content,timestamp:0}});
const entries=[message('u',null,'user','Original goal 🔍'),message('a','u','assistant',[{type:'toolCall',id:'read-1',name:'read',arguments:{path:'src/a.rs'}}]),
 {...message('t','a','toolResult',[{type:'text',text:'Huge output'.repeat(300)}]),message:{role:'toolResult',toolCallId:'read-1',toolName:'read',content:[{type:'text',text:'Huge output'.repeat(300)}],isError:false,timestamp:0}},
 {type:'branch_summary',id:'summary',parentId:'t',timestamp:now,fromId:'u',summary:'Prior branch',details:{readFiles:['a','b'],modifiedFiles:['b']}},
 message('other','t','user','Alternative request'),
 {type:'compaction',id:'compact',parentId:'summary',timestamp:now,firstKeptEntryId:'u',tokensBefore:1234,summary:'Historical checkpoint'},
 {type:'custom_message',id:'custom',parentId:'compact',timestamp:now,customType:'extension',content:[{type:'text',text:'Custom instruction'}],display:true},
 message('edit','custom','assistant',[{type:'toolCall',id:'edit-1',name:'edit',arguments:{path:'src/a.rs'}}]),
 {type:'branch_summary',id:'hook',parentId:'edit',timestamp:now,fromId:'u',fromHook:true,summary:'Extension summary',details:{readFiles:['not-native'],modifiedFiles:['not-native']}},
];
const index=new Map(entries.map(e=>[e.id,e]));
const getBranch=id=>{const result=[];while(id){const e=index.get(id);assert(e);result.unshift(e);id=e.parentId;}return result;};
const session={getBranch,getEntry:id=>index.get(id)};
const collections=[['hook','other'],['hook','u'],['u','hook'],['hook','hook'],[null,'other']].map(([old,target])=>({old,target,result:collectEntriesForBranchSummary(session,old,target)}));
const preparations=[0,1,30,100,10000,-1].map(budget=>{const result=prepareBranchEntries(entries,budget);return {budget,result:{...result,fileOps:Object.fromEntries(Object.entries(result.fileOps).map(([k,v])=>[k,[...v].sort()]))}};});
const sources=['packages/coding-agent/src/core/compaction/branch-summarization.ts','packages/coding-agent/src/core/agent-session-tree-coordinator.ts'];
const result=JSON.parse(JSON.stringify({version:1,source_commit:'e30f0b4b238ee5618928bcd17b4571eb046e7038',source_files:sources.map(file=>({path:file,sha256:createHash('sha256').update(fs.readFileSync(path.join(base,file))).digest('hex')})),entries,collections,preparations,model_actions:0,network_actions:0}));
const target=path.join(root,'fixtures/runtime/tree-oracle.json');
if(process.argv.includes('--capture')){assert(!fs.existsSync(target));fs.writeFileSync(target,JSON.stringify(result,null,2)+'\n');}else assert.deepEqual(result,JSON.parse(fs.readFileSync(target,'utf8')));
console.log(JSON.stringify({status:'passed',collections:collections.length,preparations:preparations.length,model_actions:0,network_actions:0}));
