import fs from 'node:fs';import path from 'node:path';import assert from 'node:assert/strict';import {syncBuiltinESMExports} from 'node:module';import {fileURLToPath,pathToFileURL} from 'node:url';
await import('../offline_guard.mjs');const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../..');process.env.AG1_OFFLINE='1';const dir=fs.mkdtempSync(path.join(root,'.local/p09-resource-overrides-'));const cwd=path.join(dir,'project'),agentDir=path.join(dir,'agent');
const exists=fs.existsSync;fs.existsSync=function(input){const p=String(input);if(!p.startsWith(dir+path.sep)&&(p.endsWith('/AGENTS.md')||p.endsWith('/CLAUDE.md')||p.includes('/.agents/skills')))return false;return exists(input);};syncBuiltinESMExports();
const {DefaultResourceLoader}=await import(pathToFileURL(path.join(root,'reference/ag1-executable/packages/coding-agent/dist/core/resource-loader.js')));
const files={'agent/settings.json':'{}','project/.ag1/skills/base/SKILL.md':'---\nname: base\ndescription: Base skill.\n---\nBody.','project/.ag1/prompts/base.md':'Base prompt.','project/.ag1/SYSTEM.md':'Base system.','project/.ag1/APPEND_SYSTEM.md':'Base appendix.','project/AGENTS.md':'Base context.','contrib/extra.md':'Extra prompt.'};
for(const [p,b]of Object.entries(files)){fs.mkdirSync(path.dirname(path.join(dir,p)),{recursive:true});fs.writeFileSync(path.join(dir,p),b);}fs.mkdirSync(path.join(cwd,'.git'));
const skill={name:'injected',description:'Injected skill',filePath:'/fake/path',baseDir:'/fake',sourceInfo:{path:'/fake/path',source:'custom',scope:'temporary',origin:'top-level'},disableModelInvocation:false,triggers:[]};
const rows=[];
try{for(const variant of ['skills','system','combined']){
 const calls=[];const options={cwd,agentDir,noExtensions:true};
 if(variant!=='system')options.skillsOverride=base=>{calls.push('skills');return {skills:[skill],diagnostics:base.diagnostics}};
 if(variant!=='skills')options.systemPromptOverride=base=>{calls.push('system');return variant==='system'?'Custom system prompt':base+' overridden'};
 if(variant==='combined'){
  options.promptsOverride=base=>{calls.push('prompts');return {...base,prompts:base.prompts.map(p=>({...p,content:p.content+' overridden'}))}};
  options.agentsFilesOverride=base=>{calls.push('agents');return {agentsFiles:base.agentsFiles.map(f=>({...f,content:f.content+' overridden'}))}};
  options.appendSystemPromptOverride=base=>{calls.push('append');return [...base,'Injected appendix.']};
 }
 const loader=new DefaultResourceLoader(options);await loader.reload();
 const snapshot=()=>({skills:loader.getSkills().skills,prompts:loader.getPrompts().prompts,agents:loader.getAgentsFiles().agentsFiles,system:loader.getSystemPrompt(),append:loader.getAppendSystemPrompt()});
 rows.push({variant,stage:'reload',calls:[...calls],output:snapshot()});calls.length=0;
 loader.extendResources({promptPaths:[{path:'../contrib/extra.md',metadata:{source:'extension:fixture',scope:'temporary',origin:'package',baseDir:path.join(dir,'contrib')}}]});
 rows.push({variant,stage:'extend-prompts',calls:[...calls],output:snapshot()});
 }}finally{fs.rmSync(dir,{recursive:true,force:true});}
const result=JSON.parse(JSON.stringify({version:1,source_commit:'e30f0b4b238ee5618928bcd17b4571eb046e7038',files,skill,rows}).replaceAll(dir,'/fixture'));const target=path.join(root,'fixtures/tools/resource-overrides-oracle.json');if(process.argv.includes('--capture')){assert(!exists(target));fs.writeFileSync(target,JSON.stringify(result,null,2)+'\n');}else assert.deepEqual(result,JSON.parse(fs.readFileSync(target,'utf8')));console.log(JSON.stringify({status:'passed',resource_override_cases:rows.length,network_actions:0,model_actions:0}));
