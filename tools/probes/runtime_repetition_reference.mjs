// Execute unchanged pinned repetition policy in disposable local files.
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {fileURLToPath,pathToFileURL} from 'node:url';
await import('../offline_guard.mjs');
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../..');
const base=path.join(root,'reference/ag1-executable/packages/coding-agent');
const {RepetitionGuard}=await import(pathToFileURL(path.join(base,'dist/core/repetition-guard.js')));
const execute=(id,tool='read',args={path:'src/a.ts'},text='same',error=false)=>({kind:'execute',id,tool,args,text,error});
const repeated=(tool='read',args={path:'src/a.ts'})=>Array.from({length:4},(_,i)=>execute(String(i),tool,args));
const cases=[
 ['same-read',repeated()],
 ['equivalent-ls',[execute('one','bash',{command:'ls src'}),execute('two','bash',{command:'ls ./src'}),execute('three','bash',{command:'cd $ROOT && ls src/'})]],
 ['inventory',repeated('bash',{command:'ls -la src && wc -l src/*.ts'})],
 ['alternating',Array.from({length:6},(_,i)=>execute(String(i),'read',{path:i%2?'src/b.ts':'src/a.ts'}))],
 ['shell-alternating',['grep -n \'export\' src/a.ts',"sed -n '1,2p' src/a.ts","grep -n 'export' ./src/a.ts","sed -n '1,2p' ./src/a.ts","grep -n 'export' src/a.ts","sed -n '1,2p' src/a.ts"].map((command,i)=>execute(String(i),'bash',{command}))],
 ['negative',Array.from({length:4},(_,i)=>execute(String(i),'grep',{path:'src',pattern:'absent'},'no matches',true))],
 ['new-ranges',Array.from({length:8},(_,i)=>execute(String(i),'read',{path:'src/a.ts',offset:i+1,limit:1}))],
 ['full-result',Array.from({length:5},(_,i)=>execute(String(i),'read',{path:'src/a.ts'},'x'.repeat(5000)+i))],
 ['file-edit',[...repeated().slice(0,2),{kind:'write',path:'src/a.ts',text:'changed'},execute('fresh')]],
 ['nested-edit',[...repeated('ls',{path:'src'}).slice(0,2),{kind:'write',path:'src/a.ts',text:'changed'},execute('fresh','ls',{path:'src'})]],
 ['expiry',[...repeated().slice(0,2),{kind:'time',ms:120000},execute('fresh')]],
 ['reset',[...repeated(),{kind:'reset'},execute('fresh')]],
 ['bookkeeping',[...repeated().slice(0,3),execute('todo','todo',{todos:[]}),execute('again')]],
 ['new-evidence',[...repeated().slice(0,3),execute('fresh','read',{path:'src/b.ts'},'new source'),execute('again')]],
 ['unknown-invalidates',[...repeated().slice(0,2),execute('mutation','bash',{command:'node change-files.js'}),execute('fresh')]],
 ['pending',[{...execute('pending'),kind:'before'},execute('duplicate'),{kind:'after',id:'pending'},execute('two'),execute('three')]],
 ['abandoned',[{...execute('pending'),kind:'before'},{kind:'start'},execute('fresh')]],
 ['changed-during',[{...execute('pending'),kind:'before'},{kind:'write',path:'src/a.ts',text:'changed'},{kind:'after',id:'pending'},execute('two'),execute('three')]],
 ['late-around-mutation',[{...execute('pending'),kind:'before'},{...execute('write','write',{path:'src/b.ts'}),kind:'before'},{kind:'after',id:'pending'},{kind:'after',id:'write'},execute('two'),execute('three')]],
 ['symlink',repeated('read',{path:'alias/a.ts'})],
 ['missing',repeated('read',{path:'missing'})],
 ['oversized-tree',[{kind:'tree'},...repeated('ls',{path:'src'})]],
 ['parallel-cap',[...Array.from({length:65},(_,i)=>({...execute('p'+i,'read',{path:'src/a.ts',offset:i+1}),kind:'before'})),execute('untracked','read',{path:'src/a.ts',offset:65})]],
 ...['job','check','webFetch'].map(tool=>['unbounded-'+tool,repeated(tool,{action:'poll'})]),
 ...["sed -n '1p; e date' src/a.ts","grep -n '$(date)' src/a.ts","grep -n 'export' src/a.ts | head","sed -n '1p\" src/a.ts","cd - && sed -n '1p' src/a.ts",'date','ls src; touch src/a.ts','ls src | head',"sed -n '0p' src/a.ts","sed -n '9007199254740992p' src/a.ts"].map((command,i)=>['unknown-shell-'+i,repeated('bash',{command})]),
];
const rows=[];
for(const [id,steps] of cases){
 const directory=fs.mkdtempSync(path.join(os.tmpdir(),'ag1-repetition-oracle-'));
 const cwd=path.join(directory,'workspace');
 try {
  fs.mkdirSync(path.join(cwd,'src'),{recursive:true});
  fs.writeFileSync(path.join(cwd,'src/a.ts'),'export const a = 1;\n');
  fs.writeFileSync(path.join(cwd,'src/b.ts'),'export const b = 2;\n');
  fs.symlinkSync(path.join(cwd,'src'),path.join(cwd,'alias'));
  let now=1000;
  const events=[];
  const guard=new RepetitionGuard(cwd,event=>{const {signature,...rest}=event;events.push(rest)},()=>now);
  const pending=new Map();
  const outcomes=[];
  for(const step of steps){
   if(step.kind==='reset') guard.reset();
   else if(step.kind==='start') guard.startTurn();
   else if(step.kind==='time') now+=step.ms;
   else if(step.kind==='write') fs.writeFileSync(path.join(cwd,step.path),step.text);
   else if(step.kind==='tree') for(let i=0;i<257;i++)fs.writeFileSync(path.join(cwd,'src',String(i)),'');
   else {
    const item=step.kind==='after'?pending.get(step.id):step;
    const args=JSON.parse(JSON.stringify(item.args).replaceAll('$ROOT',cwd));
    if(step.kind==='after')guard.after(step.id,{content:[{type:'text',text:item.text}]},item.error,item.tool,args);
    else {
     const block=guard.before(item.tool,item.id,args);
     outcomes.push({id:item.id,reason:block?.reason??null});
     if(!block){
      pending.set(item.id,item);
      if(step.kind==='execute')guard.after(item.id,{content:[{type:'text',text:item.text}]},item.error,item.tool,args);
     }
    }
   }
  }
  rows.push({id,steps,outcomes,events,stop:guard.getStopReason()??null});
 }finally{fs.rmSync(directory,{recursive:true,force:true});}
}
const source='packages/coding-agent/src/core/repetition-guard.ts';
const result={version:1,source_commit:'e30f0b4b238ee5618928bcd17b4571eb046e7038',source_files:[{path:source,sha256:createHash('sha256').update(fs.readFileSync(path.join(base,'src/core/repetition-guard.ts'))).digest('hex')}],rows,model_actions:0,network_actions:0};
const target=path.join(root,'fixtures/runtime/repetition-oracle.json');
if(process.argv.includes('--capture')){assert(!fs.existsSync(target));fs.writeFileSync(target,JSON.stringify(result,null,2)+'\n');}
else assert.deepEqual(result,JSON.parse(fs.readFileSync(target,'utf8')));
console.log(JSON.stringify({status:'passed',cases:rows.length,model_actions:0,network_actions:0}));
