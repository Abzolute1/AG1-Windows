import fs from 'node:fs';import path from 'node:path';import assert from 'node:assert/strict';import {fileURLToPath,pathToFileURL} from 'node:url';
await import('../offline_guard.mjs');
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../..');
const sdk=path.join(root,'reference/ag1-executable/packages/coding-agent/dist/core');
const {createExtensionRuntime,loadExtensionFromFactory}=await import(pathToFileURL(path.join(sdk,'extensions/loader.js')));
const {ExtensionRunner}=await import(pathToFileURL(path.join(sdk,'extensions/runner.js')));
const {createEventBus}=await import(pathToFileURL(path.join(sdk,'event-bus.js')));
const keybindings={'app.interrupt':['ctrl+c','CTRL+X'],'app.model.select':'ctrl+m','app.plan.review':'ctrl+p','tui.select.cancel':'escape','tui.input.submit':'enter','tui.picker.next':'ctrl+n','app.fixture':'alt+f'};
const specs=[
 {id:'empty',sources:[]},
 {id:'collisions',sources:[
 `export default api=>{api.registerTool({name:'shared',description:'first',parameters:{type:'object'},execute:async()=>({content:[]})});api.registerCommand('same',{description:'first',handler:()=> 'first'});api.registerCommand('same:1',{handler:()=> 'literal'});api.registerFlag('shared',{type:'string',default:'first'});api.registerFlag('no-default',{type:'boolean'});api.registerMessageRenderer('record',()=> 'first');api.registerShortcut('CTRL+F',{description:'first',handler:()=> 'first'});api.registerCommand('flags',{handler:()=>({shared:api.getFlag('shared'),unknown:api.getFlag('unknown')})});}`,
 `export default api=>{api.registerTool({name:'shared',description:'last',parameters:{type:'object'},execute:async()=>({content:[]})});api.registerCommand('same',{description:'last',handler:()=> 'last'});api.registerFlag('shared',{type:'string',default:'last'});api.registerFlag('second',{type:'boolean',default:false});api.registerMessageRenderer('record',()=> 'last');api.registerShortcut('ctrl+f',{description:'last',handler:()=> 'last'});}`]},
 {id:'reserved-and-picker',sources:[`export default api=>{for(const key of ['ctrl+c','ctrl+x','ctrl+m','ctrl+p','escape','enter','ctrl+n','alt+f','alt+z'])api.registerShortcut(key,{description:key,handler:()=>key});}`]},
 {id:'intra-extension-overwrite',sources:[`export default api=>{api.registerTool({name:'same',description:'old',parameters:{type:'object'},execute:async()=>({content:[]})});api.registerTool({name:'same',description:'new',parameters:{type:'object'},execute:async()=>({content:[]})});api.registerCommand('same',{description:'old',handler:()=> 'old'});api.registerCommand('same',{description:'new',handler:()=> 'new'});api.registerFlag('same',{type:'string',default:'old'});api.registerFlag('same',{type:'string',default:'new'});api.registerShortcut('alt+z',{description:'old',handler:()=> 'old'});api.registerShortcut('alt+z',{description:'new',handler:()=> 'new'});}`]},
];
const clean=v=>JSON.parse(JSON.stringify(v)),rows=[];
for(const spec of specs){
 const runtime=createExtensionRuntime(),extensions=[];
 for(const [i,source]of spec.sources.entries())extensions.push(await loadExtensionFromFactory((await import(`data:text/javascript,${encodeURIComponent(source)}`)).default,'/fixture',createEventBus(),runtime,`/fixture/extension-${i}.ts`));
 const runner=new ExtensionRunner(extensions,runtime,'/fixture',{},{});
 const previous=console.warn;console.warn=()=>{};let shortcuts;try{shortcuts=[...runner.getShortcuts(keybindings)].map(([key,s])=>({key,shortcut:s.shortcut,description:s.description,extensionPath:s.extensionPath}));}finally{console.warn=previous;}
 const commands=runner.getRegisteredCommands().map(c=>({name:c.name,invocationName:c.invocationName,description:c.description}));
 const commandResults=[];for(const c of runner.getRegisteredCommands())commandResults.push({name:c.invocationName,result:await c.handler('',runner.createCommandContext())});
 const tools=runner.getAllRegisteredTools().map(t=>({name:t.definition.name,description:t.definition.description}));
 const flags=[...runner.getFlags().values()];const defaults=Object.fromEntries(runner.getFlagValues());runner.setFlagValue('shared','overridden');runner.setFlagValue('unknown',true);
 const values=Object.fromEntries(runner.getFlagValues());const copied=runner.getFlagValues();copied.set('shared','mutated-copy');assert.equal(runner.getFlagValues().get('shared'),'overridden');
 const flagCommand=runner.getCommand('flags');const updatedFlags=flagCommand?await flagCommand.handler('',runner.createCommandContext()):null;
 const renderer=runner.getMessageRenderer('record');
 rows.push({...spec,keybindings,output:clean({tools,commands,commandResults,flags,defaults,values,updatedFlags,shortcuts,diagnostics:runner.getShortcutDiagnostics(),renderer:renderer?renderer({}, {}, {}):null})});
}
const result={version:1,source_commit:'e30f0b4b238ee5618928bcd17b4571eb046e7038',rows};const target=path.join(root,'fixtures/tools/extension-collections-oracle.json');if(process.argv.includes('--capture')){assert(!fs.existsSync(target));fs.writeFileSync(target,JSON.stringify(result,null,2)+'\n');}else assert.deepEqual(result,JSON.parse(fs.readFileSync(target,'utf8')));
console.log(JSON.stringify({status:'passed',extension_collection_cases:rows.length,network_actions:0,model_actions:0}));
