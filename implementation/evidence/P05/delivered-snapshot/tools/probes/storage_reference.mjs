// Frozen persistence corpus and observable oracle from the immutable P01 SDK.
// Writes only fixtures/storage and disposable copies. Never loads a live user journal.
import fs from 'node:fs';import path from 'node:path';import os from 'node:os';
import {createHash,createHmac} from 'node:crypto';import {pathToFileURL} from 'node:url';
const root=path.resolve(import.meta.dirname,'../..'),reference=path.join(root,'reference/ag1-executable');
const {ArtifactStore}=await import(pathToFileURL(reference+'/packages/coding-agent/dist/core/artifact-store.js'));
const {migrateSessionEntries,SessionManager}=await import(pathToFileURL(reference+'/packages/coding-agent/dist/core/session-manager.js'));
const {loadMemoryStore,memoryStorePath}=await import(pathToFileURL(reference+'/packages/coding-agent/dist/core/memory-store.js'));
const base=path.join(root,'fixtures/storage'),source=path.join(base,'source');
const sha=bytes=>createHash('sha256').update(bytes).digest('hex');
const write=(relative,bytes)=>{const p=path.join(source,relative);fs.mkdirSync(path.dirname(p),{recursive:true});fs.writeFileSync(p,bytes);};
const json=(relative,value)=>write(relative,JSON.stringify(value,null,2)+'\n');
const canonical=entries=>{const ids=new Map(entries.filter(e=>e.type!=='session').map((e,i)=>[e.id,`entry-${i+1}`]));return entries.map(e=>{e=structuredClone(e);for(const k of ['id','parentId','firstKeptEntryId','fromId','targetId'])if(ids.has(e[k]))e[k]=ids.get(e[k]);return e;});};
const session=(id,version,entries)=>[{type:'session',id,version,timestamp:'2026-09-05T00:00:00.000Z',cwd:'/frozen/project',unknownHeader:{preserve:true}},...entries];
if(process.argv.includes('--capture')){
 if(fs.existsSync(base))throw Error('Storage fixtures already exist; refusing silent recapture');fs.mkdirSync(source,{recursive:true});
 const text='Å line one\r\nneedle β second line\r\nemoji 🦙 final\n';
 const dir=path.join(source,'artifacts/v3');const artifacts=new ArtifactStore(dir,'v3',{features:{typedEvidenceCursors:true,evidenceLeases:true,segmentedArtifactCatalog:true},now:()=>1_788_566_400_000,catalogSegmentRecords:2});
 const records=[];
 for(const kind of ['text','source-capsule','episode-trajectory','compaction-trajectory'])records.push(artifacts.putText(kind==='text'?text:JSON.stringify({kind,content:'historical needle evidence'}),{kind,producer:{type:'system',name:'P05 oracle'},parentEntryId:'a'}));
 const png=Buffer.alloc(24);Buffer.from([137,80,78,71,13,10,26,10]).copy(png);png.writeUInt32BE(1,16);png.writeUInt32BE(1,20);
 records.push(artifacts.putBinary(png,'image/png',{kind:'godot-visual',producer:{type:'system',name:'P05 oracle'},metadata:{width:1,height:1,purpose:'fixture',source:'frozen',retentionState:'retained'}}));
 const cursor=artifacts.createReadCursor(records[0].id,1,8);
 const encoded=Buffer.from(JSON.stringify({v:1,s:'v3',i:records[0].id,k:'context',o:0,m:8,l:1})).toString('base64url');
 const legacy='ac1.'+encoded+'.'+createHmac('sha256',fs.readFileSync(path.join(dir,'cursor-key-v1'))).update(encoded).digest('base64url');
 artifacts.leaseEvidence(records[0].id,'retain historical context',{ttlMs:60000});
 const leaf=[{type:'model_change',id:'a',parentId:null,timestamp:'t',provider:'fixture',modelId:'preserved-model'},
 {type:'thinking_level_change',id:'b',parentId:'a',timestamp:'t',thinkingLevel:'high'},
 {type:'custom',id:'c',parentId:'b',timestamp:'t',customType:'opaque-extension',data:{artifacts:records.map(r=>r.id),cursor,legacy,unknown:{nested:[1,null,'18446744073709551615']}}},
 {type:'message',id:'side',parentId:'a',timestamp:'t',message:{role:'user',content:'physical sibling',timestamp:0}},
 {type:'custom_message',id:'d',parentId:'c',timestamp:'t',customType:'notice',content:'visible extension',display:true,details:{untouched:true}},
 {type:'compaction',id:'e',parentId:'d',timestamp:'t',summary:'retained evidence',firstKeptEntryId:'d',tokensBefore:42,details:{trajectory:records[3].id}},
 {type:'branch_summary',id:'f',parentId:'e',timestamp:'t',fromId:'side',summary:'branch summary',details:{capsule:records[1].id}},
 {type:'label',id:'g',parentId:'f',timestamp:'t',targetId:'c',label:'bookmark'},
 {type:'session_info',id:'h',parentId:'g',timestamp:'t',name:'Frozen session'}];
 write('journals/v3.jsonl',session('v3',3,leaf).map(e=>JSON.stringify(e)).join('\r\n')+'\r\n');
 const v2=[{type:'message',id:'a2',parentId:null,timestamp:'t',message:{role:'hookMessage',customType:'plugin',content:'old hook role',display:true,timestamp:0}},{type:'custom',id:'b2',parentId:'a2',timestamp:'t',customType:'unknown',data:{preserved:'value'}}];
 write('journals/v2.jsonl',session('v2',2,v2).map(e=>JSON.stringify(e)).join('\n')+'\n');
 const v1=[{type:'message',timestamp:'t',message:{role:'user',content:'legacy',timestamp:0}},{type:'compaction',timestamp:'t',summary:'legacy summary',firstKeptEntryIndex:1,tokensBefore:12}];
 write('journals/v1.jsonl',session('v1',1,v1).map(e=>JSON.stringify(e)).join('\n')+'\n');
 const p01=JSON.parse(fs.readFileSync(path.join(root,'fixtures/traces/session.json'))).scenarios[0].journal;
 write('journals/p01.jsonl',session('p01',3,p01).map(e=>JSON.stringify(e)).join('\n')+'\n');
 for(const version of [1,2,3])json(`memory/v${version}.json`,{version,entries:[{id:'m1',kind:'decision',text:'Keep tuned runtimes',createdAt:123,...(version>=2?{priority:'core'}:{}),...(version===3?{component:'runtime',sources:['file:runtime@sha256']}:{}),extension:{retained:true}}],...(version===3?{archived:[{id:'m2',kind:'style',text:'archived memory',createdAt:122}]}:{}),unknownTop:{preserved:'yes'}});
 write('memory/corrupt.json','{"version":3,"entries":[');write('memory/empty.json','');json('memory/invalid.json',{version:3,entries:[{id:7}]});
 json('metadata/checkpoint.json',{id:'ckpt_frozen',type:'verify',description:'frozen completed turn',startedAt:'2026-09-05T00:00:00Z',updatedAt:'2026-09-05T00:00:01Z',evidence:[{kind:'file',path:'README.md',turn:1,at:'2026-09-05T00:00:01Z'}],summaries:[],usage:{input:1,output:1,totalTokens:2,cost:0},lastTurn:1,toolCalls:1,budgetPressure:'low',extension:{preserved:true}});
 json('metadata/jobs.json',{version:1,jobs:[{id:'job_frozen',command:'never execute imported metadata',pid:12345,pidStart:'67890',cwd:'/frozen/project',startedAt:1,logPath:'job.log',exitPath:'job.exit',status:'running'}],unknown:true});
 json('metadata/runtime.json',{pid:12345,serverPath:'/historical/llama-server',authority:'historical observation only'});
 const files=[];
 const walk=dir=>{for(const e of fs.readdirSync(dir,{withFileTypes:true})){const p=path.join(dir,e.name);if(e.isDirectory())walk(p);else{const relative=path.relative(source,p),bytes=fs.readFileSync(p);let kind='extra',session=null,origin=null;
 if(relative.startsWith('journals/')){kind='journal';session=path.basename(p,'.jsonl');}
 else if(relative.startsWith('memory/'))kind='memory';else if(relative.startsWith('metadata/'))kind='metadata';
 else if(relative.startsWith('artifacts/')){session='v3';origin='v3';if(relative.includes('/records/'))kind='artifact';else if(relative.includes('/blobs/'))kind='blob';else if(e.name==='cursor-key-v1')kind='cursor_key';else if(e.name==='evidence-leases-v1.json')kind='leases';}
 files.push({path:relative,sha256:sha(bytes),bytes:bytes.length,kind,session,origin});}}};walk(source);files.sort((a,b)=>a.path.localeCompare(b.path));
 fs.writeFileSync(path.join(base,'manifest.json'),JSON.stringify({version:1,frozen:true,files},null,2)+'\n');
 fs.writeFileSync(path.join(base,'identities.json'),JSON.stringify({records:records.map(r=>r.id),text,cursor,legacy},null,2)+'\n');
}
const manifest=JSON.parse(fs.readFileSync(path.join(base,'manifest.json')));
for(const f of manifest.files){const bytes=fs.readFileSync(path.join(source,f.path));if(sha(bytes)!==f.sha256||bytes.length!==f.bytes)throw Error('Frozen source drift: '+f.path);}
const scratch=fs.mkdtempSync(path.join(root,'.local/p05-oracle-'));
try{
 const observed={journals:{},memory:{},pages:[],cursors:{},model_actions:0};
 for(const f of manifest.files.filter(f=>f.kind==='journal')){const lines=fs.readFileSync(path.join(source,f.path),'utf8').trim().split(/\r?\n/).map(JSON.parse);migrateSessionEntries(lines);observed.journals[f.session]=canonical(lines);}
 for(const f of manifest.files.filter(f=>f.kind==='memory'&&!/corrupt|empty|invalid/.test(f.path))){const cwd=path.join(scratch,sha(f.path));fs.mkdirSync(path.dirname(memoryStorePath(cwd)),{recursive:true});fs.copyFileSync(path.join(source,f.path),memoryStorePath(cwd));observed.memory[f.path]=loadMemoryStore(cwd);}
 const dir=path.join(scratch,'artifacts');fs.cpSync(path.join(source,'artifacts/v3'),dir,{recursive:true});
 const artifacts=new ArtifactStore(dir,'v3',{features:{typedEvidenceCursors:true,evidenceLeases:true,segmentedArtifactCatalog:true},now:()=>1_788_566_400_000});
 const ids=JSON.parse(fs.readFileSync(path.join(base,'identities.json')));
 for(let offset=0;offset<Buffer.byteLength(ids.text)+2;offset++){const p=artifacts.read(ids.records[0],offset,5);observed.pages.push({offset:p.offset,text:p.text,nextOffset:p.nextOffset??null});}
 for(const name of ['cursor','legacy']){const p=artifacts.readCursor(ids[name]);observed.cursors[name]={text:p.text,offset:p.offset,nextOffset:p.nextOffset??null};}
 observed.search=(await artifacts.search('needle')).matches.map(m=>m.record?.id??m.artifactId??m.id);
 const expected=path.join(base,'oracle.json');
 if(process.argv.includes('--capture'))fs.writeFileSync(expected,JSON.stringify(observed,null,2)+'\n');
 else if(JSON.stringify(JSON.parse(fs.readFileSync(expected)))!==JSON.stringify(observed))throw Error('Reference observable drift');
 console.log(JSON.stringify({status:'passed',source_files:manifest.files.length,journals:Object.keys(observed.journals).length,utf8_windows:observed.pages.length,cursor_versions:2,model_actions:0,source_commit:'e30f0b4b238ee5618928bcd17b4571eb046e7038'}));
}finally{fs.rmSync(scratch,{recursive:true,force:true});}
