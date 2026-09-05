import fs from 'node:fs';
import path from 'node:path';
import {execFileSync} from 'node:child_process';
import assert from 'node:assert/strict';
import {fileURLToPath,pathToFileURL} from 'node:url';
await import('../offline_guard.mjs');
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../..');
const cwd=fs.mkdtempSync(path.join(root,'.local/p09-git-'));
const {createDiffToolDefinition}=await import(pathToFileURL(path.join(root,'reference/ag1-executable/packages/coding-agent/dist/core/tools/diff-tool.js')));
const env={PATH:'/usr/bin:/bin',HOME:cwd,GIT_CONFIG_NOSYSTEM:'1',GIT_CONFIG_GLOBAL:'/dev/null',GIT_AUTHOR_NAME:'Fixture',GIT_AUTHOR_EMAIL:'fixture@localhost',GIT_COMMITTER_NAME:'Fixture',GIT_COMMITTER_EMAIL:'fixture@localhost',GIT_AUTHOR_DATE:'2020-01-01T00:00:00Z',GIT_COMMITTER_DATE:'2020-01-01T00:00:00Z',LC_ALL:'C'};
const git=(...args)=>execFileSync('/usr/bin/git',args,{cwd,env,encoding:'utf8',stdio:['ignore','pipe','pipe']});
const initial={'a.txt':'one\ntwo\nthree\n','b.txt':'first\nsecond\n','src/multi.ts':Array.from({length:40},(_,i)=>`line ${i+1}`).join('\n')+'\n','space file.txt':'old\n'};
const changed={'a.txt':'one\nTWO\nthree\nnew\n','b.txt':'FIRST\nsecond\n','src/multi.ts':initial['src/multi.ts'].replace('line 3\n','changed 3\n').replace('line 35\n','changed 35\n'),'space file.txt':'new\n'};
function write(files){for(const [name,text]of Object.entries(files)){fs.mkdirSync(path.dirname(path.join(cwd,name)),{recursive:true});fs.writeFileSync(path.join(cwd,name),text);}}
const rows=[];
try {
 git('init','-b','main');write(initial);git('add','.');git('commit','-m','fixture');git('branch','base');
 write(changed);
 const inputs=[{}, {from:'a.txt'},{from:'a.txt',to:'b.txt'},{againstBranch:'base'},{againstCommit:'HEAD'},{againstCommit:'HEAD',paths:['a.txt']},{againstCommit:'HEAD',paths:['space file.txt']},{from:'src/multi.ts',maxChanges:2},{from:'src/multi.ts',maxChanges:3},{from:'a.txt',maxChanges:1},{from:'missing.txt'},{from:'space file.txt'},{againstCommit:'HEAD',paths:[]},{againstBranch:'missing'}];
 const definition=createDiffToolDefinition(cwd);
 for(const [i,input]of inputs.entries()){
   const output=await definition.execute(`diff-${i}`,input);
   let args;
   if(input.againstBranch)args=['diff',`HEAD...${input.againstBranch}`];
   else if(input.againstCommit)args=['diff',input.againstCommit,...(input.paths?['--',...input.paths]:[])];
   else args=['diff','--',String(input.from),...(input.to?['--',input.to]:[])];
   let raw='';try{raw=git(...args);}catch{}
   rows.push({input,output,raw});
 }
}finally{fs.rmSync(cwd,{recursive:true,force:true});}
const result={version:1,source_commit:'e30f0b4b238ee5618928bcd17b4571eb046e7038',initial,changed,rows,model_actions:0,network_actions:0};
const target=path.join(root,'fixtures/tools/git-oracle.json');
if(process.argv.includes('--capture')){assert(!fs.existsSync(target));fs.writeFileSync(target,JSON.stringify(result,null,2)+'\n');}else assert.deepEqual(result,JSON.parse(fs.readFileSync(target,'utf8')));
console.log(JSON.stringify({status:'passed',git_cases:rows.length,model_actions:0,network_actions:0}));
