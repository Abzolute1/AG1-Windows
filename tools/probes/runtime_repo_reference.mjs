// Actual pinned repo tool, local disposable Git, no network/model calls.
import fs from 'node:fs';import os from 'node:os';import path from 'node:path';
import assert from 'node:assert/strict';import {execFileSync} from 'node:child_process';
import {createHash} from 'node:crypto';import {fileURLToPath,pathToFileURL} from 'node:url';
await import('../offline_guard.mjs');
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../..');
const base=path.join(root,'reference/ag1-executable/packages/coding-agent');
const {createRepoToolDefinition}=await import(pathToFileURL(path.join(base,'dist/core/tools/repo.js')));
const files={'package.json':'{"name":"owned-project","version":"1.2.3","main":"src/main.ts","bin":{"cli":"bin/cli.js"}}\n','Cargo.toml':'[package]\nname = "native-core"\nversion = "0.1.0"\n','src/main.ts':'export const answer = 42;\n','README.md':'# Owned project\n\nRepository orientation fixture.\n'};
const memory={version:3,entries:[{id:'m1',kind:'decision',text:'Preserve tuned runtimes',createdAt:1000,priority:'core'}]};
const temporary=fs.mkdtempSync(path.join(os.tmpdir(),'ag1-repo-tool-'));const rows=[];
// The isolated TMPDIR may itself live under this checkout. The nonrepo case
// must stop Git discovery at its fixture boundary, not observe the parent repo.
const previousCeiling=process.env.GIT_CEILING_DIRECTORIES;
process.env.GIT_CEILING_DIRECTORIES=path.dirname(temporary);
const env={...process.env,GIT_CONFIG_NOSYSTEM:'1',GIT_CONFIG_GLOBAL:'/dev/null',GIT_AUTHOR_NAME:'Fixture',GIT_AUTHOR_EMAIL:'fixture@example.invalid',GIT_COMMITTER_NAME:'Fixture',GIT_COMMITTER_EMAIL:'fixture@example.invalid',GIT_AUTHOR_DATE:'2026-09-06T00:00:00Z',GIT_COMMITTER_DATE:'2026-09-06T00:00:00Z',LC_ALL:'C'};
try {
 for(const [name,text] of Object.entries(files)){const target=path.join(temporary,name);fs.mkdirSync(path.dirname(target),{recursive:true});fs.writeFileSync(target,text);}
 fs.mkdirSync(path.join(temporary,'.ag1'));fs.writeFileSync(path.join(temporary,'.ag1/memory.json'),JSON.stringify(memory));
 const git=(...args)=>execFileSync('/usr/bin/git',args,{cwd:temporary,env,stdio:['ignore','pipe','pipe']});
 const capture=async state=>{for(const view of ['status','map','all']) rows.push({state,view,expected:await createRepoToolDefinition(temporary).execute('owned-repo',{view})});};
 await capture('nonrepo');
 git('init','-b','main');git('config','core.abbrev','7');git('add','package.json','Cargo.toml','src/main.ts','README.md');git('commit','-m','Owned baseline');
 fs.writeFileSync(path.join(temporary,'src/main.ts'),'export const answer = 43;\n');fs.writeFileSync(path.join(temporary,'new.txt'),'untracked\n');
 await capture('dirty');
}finally{fs.rmSync(temporary,{recursive:true,force:true});if(previousCeiling===undefined)delete process.env.GIT_CEILING_DIRECTORIES;else process.env.GIT_CEILING_DIRECTORIES=previousCeiling;}
const result={version:1,source_commit:'e30f0b4b238ee5618928bcd17b4571eb046e7038',source_files:['repo-map','tools/repo','memory-store'].map(name=>({path:`packages/coding-agent/src/core/${name}.ts`,sha256:createHash('sha256').update(fs.readFileSync(path.join(base,`src/core/${name}.ts`))).digest('hex')})),files,memory,rows,model_actions:0,network_actions:0};
const target=path.join(root,'fixtures/runtime/repo-oracle.json');
if(process.argv.includes('--capture')){assert(!fs.existsSync(target));fs.writeFileSync(target,JSON.stringify(result,null,2)+'\n');}else assert.deepEqual(JSON.parse(JSON.stringify(result)),JSON.parse(fs.readFileSync(target,'utf8')));
console.log(JSON.stringify({status:'passed',repo_tool_cases:rows.length,model_actions:0,network_actions:0}));
