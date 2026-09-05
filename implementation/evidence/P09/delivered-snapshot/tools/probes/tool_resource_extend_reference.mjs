import fs from 'node:fs';import path from 'node:path';import assert from 'node:assert/strict';import {syncBuiltinESMExports} from 'node:module';import {fileURLToPath,pathToFileURL} from 'node:url';
await import('../offline_guard.mjs');const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../..');process.env.AG1_OFFLINE='1';const fixture=fs.mkdtempSync(path.join(root,'.local/p09-resource-extend-'));const cwd=path.join(fixture,'project'),agentDir=path.join(fixture,'agent');
const exists=fs.existsSync;fs.existsSync=function(input){const p=String(input);if(!p.startsWith(fixture+path.sep)&&(p.endsWith('/AGENTS.md')||p.endsWith('/CLAUDE.md')||p.includes('/.agents/skills')))return false;return exists(input);};syncBuiltinESMExports();
const {DefaultResourceLoader}=await import(pathToFileURL(path.join(root,'reference/ag1-executable/packages/coding-agent/dist/core/resource-loader.js')));
const files={
 'project/.ag1/skills/base/SKILL.md':'---\nname: base\ndescription: Base skill.\n---\nBase body.',
 'project/.ag1/prompts/base.md':'Base prompt.',
 'project/.ag1/SYSTEM.md':'Original system.',
 'project/AGENTS.md':'Original context.',
 'agent/settings.json':'{}',
 'contrib/skills/extra/SKILL.md':'---\nname: extra\ndescription: Extra skill.\n---\nExtra body.',
 'contrib/prompts/extra.md':'Extra prompt.',
 'contrib/themes/extra.json':JSON.stringify({...JSON.parse(fs.readFileSync(path.join(root,'fixtures/tools/theme-oracle.json'),'utf8')).rows[0].input,name:'extra'}),
};
for(const [p,b]of Object.entries(files)){fs.mkdirSync(path.dirname(path.join(fixture,p)),{recursive:true});fs.writeFileSync(path.join(fixture,p),b);}fs.mkdirSync(path.join(cwd,'.git'),{recursive:true});
const normalize=v=>JSON.parse(JSON.stringify(v).replaceAll(fixture,'/fixture'));const rows=[];
const metadata=(source)=>({source,scope:'temporary',origin:'package',baseDir:path.join(fixture,'contrib')});
const actions=[
 {name:'empty',paths:{}},
 {name:'skill-directory',paths:{skillPaths:[{path:'../contrib/skills',metadata:metadata('extension:first')}]}},
 {name:'prompt-only-after-disk-changes',writes:{'project/.ag1/skills/base/SKILL.md':'---\nname: base\ndescription: Changed skill.\n---\nChanged.','project/.ag1/SYSTEM.md':'Changed system.','project/AGENTS.md':'Changed context.'},paths:{promptPaths:[{path:' ../contrib/prompts ',metadata:metadata('extension:first')}]}},
 {name:'theme-only',paths:{themePaths:[{path:'../contrib/themes',metadata:metadata('extension:first')}]}},
 {name:'skill-metadata-replacement',paths:{skillPaths:[{path:'../contrib/skills',metadata:metadata('extension:second')}]}},
 {name:'existing-prompt-metadata',paths:{promptPaths:[{path:'.ag1/prompts/base.md',metadata:metadata('extension:second')}]}},
 {name:'missing-resource',paths:{promptPaths:[{path:'../absent.md',metadata:metadata('extension:second')}]}},
 {name:'reload',reload:true},
];
try{
 const loader=new DefaultResourceLoader({cwd,agentDir,noExtensions:true});await loader.reload();
 const snapshot=()=>({skills:loader.getSkills(),prompts:loader.getPrompts(),themes:loader.getThemes().themes.map(t=>({name:t.name,sourcePath:t.sourcePath,sourceInfo:t.sourceInfo})),themeDiagnostics:loader.getThemes().diagnostics,agents:loader.getAgentsFiles(),system:loader.getSystemPrompt(),append:loader.getAppendSystemPrompt()});
 rows.push({name:'initial',output:snapshot()});
 for(const action of actions){for(const [p,b]of Object.entries(action.writes??{}))fs.writeFileSync(path.join(fixture,p),b);if(action.reload)await loader.reload();else loader.extendResources(action.paths);rows.push({...action,output:snapshot()});}
}finally{fs.rmSync(fixture,{recursive:true,force:true});}
const result=normalize({version:1,source_commit:'e30f0b4b238ee5618928bcd17b4571eb046e7038',files,rows});const target=path.join(root,'fixtures/tools/resource-extend-oracle.json');if(process.argv.includes('--capture')){assert(!exists(target));fs.writeFileSync(target,JSON.stringify(result,null,2)+'\n');}else assert.deepEqual(result,JSON.parse(fs.readFileSync(target,'utf8')));console.log(JSON.stringify({status:'passed',resource_extend_cases:rows.length,network_actions:0,model_actions:0}));
