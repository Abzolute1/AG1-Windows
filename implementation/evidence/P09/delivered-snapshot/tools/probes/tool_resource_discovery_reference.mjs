import fs from 'node:fs';import path from 'node:path';import assert from 'node:assert/strict';import {syncBuiltinESMExports} from 'node:module';import {fileURLToPath,pathToFileURL} from 'node:url';
await import('../offline_guard.mjs');const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../..');process.env.AG1_OFFLINE='1';const fixture=fs.mkdtempSync(path.join(root,'.local/p09-resources-'));const cwd=path.join(fixture,'project');const agentDir=path.join(fixture,'agent');fs.mkdirSync(path.join(cwd,'.git'),{recursive:true});fs.mkdirSync(agentDir,{recursive:true});
// The reference walks ancestor context and HOME/.agents directories. Suppress only
// those out-of-fixture resource lookups; HOME itself and source modules are unchanged.
const exists=fs.existsSync;fs.existsSync=function(input){const value=String(input);if(!value.startsWith(fixture+path.sep)&&(value.endsWith('/AGENTS.md')||value.endsWith('/CLAUDE.md')||value.includes('/.agents/skills')))return false;return exists(input);};syncBuiltinESMExports();
const base=path.join(root,'reference/ag1-executable/packages/coding-agent/dist/core');const skills=await import(pathToFileURL(path.join(base,'skills.js')));const prompts=await import(pathToFileURL(path.join(base,'prompt-templates.js')));const {DefaultResourceLoader}=await import(pathToFileURL(path.join(base,'resource-loader.js')));const {DefaultPackageManager}=await import(pathToFileURL(path.join(base,'package-manager.js')));const {SettingsManager}=await import(pathToFileURL(path.join(base,'settings-manager.js')));const {createSkill}=await import(pathToFileURL(path.join(base,'skill-creator.js')));
const files={
 'agent/skills/same/SKILL.md':'---\nname: same\ndescription: User copy.\n---\nUser instructions.',
 'project/.ag1/skills/same/SKILL.md':'---\nname: same\ndescription: Project copy.\n---\nProject instructions.',
 'project/.ag1/skills/root.md':'---\nname: root-flat\ndescription: A flat skill in the root.\n---\nRoot body.',
 'project/.ag1/skills/group/deep/SKILL.md':'---\nname: deep\ndescription: Deep skill.\ntriggers: [deep work, true, "  "]\n---\nDeep body.',
 'project/.ag1/skills/group/ignored.md':'---\ndescription: Nested loose markdown is not a skill root.\n---\nignored',
 'project/.ag1/skills/stop/SKILL.md':'---\nname: stop\ndescription: Root takes precedence.\n---\nroot',
 'project/.ag1/skills/stop/child/SKILL.md':'---\nname: child\ndescription: Never visited.\n---\nchild',
 'project/.ag1/skills/.hidden/SKILL.md':'---\ndescription: Hidden.\n---\nhidden',
 'project/.ag1/skills/node_modules/pkg/SKILL.md':'---\ndescription: Dependency.\n---\ndep',
 'project/.ag1/skills/blocked/SKILL.md':'---\ndescription: Ignored by rules.\n---\nblocked',
 'project/.ag1/skills/.gitignore':'blocked/\n',
 'project/.ag1/skills/missing/SKILL.md':'---\nname: missing\n---\nNo description.',
 'project/.ag1/skills/invalid/SKILL.md':'---\nname: Invalid--Name-\ndescription: Warnings still permit loading.\n---\nBody',
 'project/.agents/skills/agent-only/SKILL.md':'---\nname: agent-only\ndescription: Discovered in agents skill root.\n---\nAgents body.',
 'project/.agents/skills/ignored-flat.md':'---\ndescription: Agents mode skips loose markdown.\n---\nignored',
 'agent/prompts/same.md':'---\ndescription: User prompt\n---\nUser $1',
 'project/.ag1/prompts/same.md':'---\ndescription: Project prompt\n---\nProject $1',
 'project/.ag1/prompts/deep/skip.md':'Nested prompt is not auto-discovered.',
 'project/.ag1/prompts/.hidden.md':'Hidden auto prompt.',
 'project/.ag1/prompts/ignored.md':'Ignored auto prompt.',
 'project/.ag1/prompts/.ignore':'ignored.md\n',
 'project/.ag1/prompts/plain.md':'First line as a description.\nBody $ARGUMENTS',
 'agent/AGENTS.md':'Global context fixture.',
 'project/CLAUDE.md':'Project fallback context fixture.',
 'project/.ag1/SYSTEM.md':'Project system prompt fixture.',
 'agent/SYSTEM.md':'Global system prompt fixture.',
 'project/.ag1/APPEND_SYSTEM.md':'Project append fixture.',
 'agent/APPEND_SYSTEM.md':'Global append fixture.',
 'package/skills/package-skill/SKILL.md':'---\nname: package-skill\ndescription: Package skill.\n---\nPackage body.',
 'package/prompts/package-prompt.md':'Package prompt $1.',
 'package/prompts/disabled.md':'Explicitly disabled package prompt.',
 'package/package.json':JSON.stringify({name:'fixture-package',version:'1.0.0',ag1:{skills:['skills'],prompts:['prompts','!disabled.md']}}),
 'explicit/custom.md':'---\ndescription: Explicit template fixture\n---\nExplicit $@',
 'agent/settings.json':JSON.stringify({packages:[path.join(fixture,'package')]}),
 'project/.ag1/settings.json':JSON.stringify({packages:[{source:path.join(fixture,'package'),prompts:['package-prompt.md']}]}),
};
for(const [name,body]of Object.entries(files)){const target=path.join(fixture,name);fs.mkdirSync(path.dirname(target),{recursive:true});fs.writeFileSync(target,body);}
fs.symlinkSync(path.join(cwd,'.ag1/skills/same/SKILL.md'),path.join(cwd,'.ag1/skills/alias.md'));
const rows=[];const normalize=value=>JSON.parse(JSON.stringify(value).replaceAll(fixture,'/fixture'));
try{
 rows.push({name:'loadSkillsDefaults',output:skills.loadSkills({cwd,agentDir})});
 rows.push({name:'loadSkillsExplicit',output:skills.loadSkills({cwd,agentDir,includeDefaults:false,skillPaths:[path.join(cwd,'.ag1/skills'),path.join(agentDir,'skills'),path.join(fixture,'missing.md')]})});
 rows.push({name:'loadPromptsDefaults',output:prompts.loadPromptTemplates({cwd,agentDir})});
 const settings=SettingsManager.create(cwd,agentDir);const packages=new DefaultPackageManager({cwd,agentDir,settingsManager:settings});rows.push({name:'resolvePackageAndAutoResources',output:await packages.resolve()});
 const loader=new DefaultResourceLoader({cwd,agentDir,noExtensions:true,additionalPromptTemplatePaths:[path.join(fixture,'explicit/custom.md')]});await loader.reload();
 const snapshot=()=>({skills:loader.getSkills(),prompts:loader.getPrompts(),themes:loader.getThemes(),agents:loader.getAgentsFiles(),system:loader.getSystemPrompt(),append:loader.getAppendSystemPrompt(),extensionErrors:loader.getExtensions().errors});rows.push({name:'fullLoader',output:snapshot()});
 fs.writeFileSync(path.join(cwd,'.ag1/skills/same/SKILL.md'),'---\nname: same\ndescription: Project copy reloaded.\n---\nChanged body.');await loader.reload();rows.push({name:'reloadedLoader',output:snapshot()});
 for(const [name,description]of [['created-skill','A simple skill.'],['created-skill','Duplicate'],['quoted-skill','Use colon: and newline\nname: injected'],['Upper','Bad name'],['empty-desc','']]){const result=createSkill({name,description,cwd});rows.push({name:'createSkill',input:{name,description},output:result,bytes:result.path?fs.readFileSync(result.path,'utf8'):null});}
}catch(error){console.error(error);process.exitCode=1;throw error;}finally{fs.rmSync(fixture,{recursive:true,force:true});}
const result=normalize({version:1,source_commit:'e30f0b4b238ee5618928bcd17b4571eb046e7038',files,symlinks:[{path:'project/.ag1/skills/alias.md',target:'project/.ag1/skills/same/SKILL.md'}],rows});const target=path.join(root,'fixtures/tools/resource-discovery-oracle.json');if(process.argv.includes('--capture')){assert(!fs.existsSync(target));fs.writeFileSync(target,JSON.stringify(result,null,2)+'\n');}else assert.deepEqual(result,JSON.parse(fs.readFileSync(target,'utf8')));
console.log(JSON.stringify({status:'passed',resource_discovery_cases:rows.length,fixture_files:Object.keys(files).length,model_actions:0,network_actions:0}));
