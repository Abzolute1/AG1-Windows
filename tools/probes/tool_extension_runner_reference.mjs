// Runs only owned inline factories against the pinned compiled SDK. No session,
// model registry, credentials or external processes are constructed by this probe.
import fs from 'node:fs';
import path from 'node:path';
import assert from 'node:assert/strict';
import {fileURLToPath,pathToFileURL} from 'node:url';
await import('../offline_guard.mjs');
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../..');
const sdk=path.join(root,'reference/ag1-executable/packages/coding-agent/dist/core');
const {createExtensionRuntime,loadExtensionFromFactory}=await import(pathToFileURL(path.join(sdk,'extensions/loader.js')));
const {ExtensionRunner}=await import(pathToFileURL(path.join(sdk,'extensions/runner.js')));
const {createEventBus}=await import(pathToFileURL(path.join(sdk,'event-bus.js')));
const clean=value=>JSON.parse(JSON.stringify(value));
const packet=value=>value===undefined?{defined:false}:{defined:true,value:clean(value)};
const cases=[];
function add(id,event,handlers){cases.push({id,event,sources:handlers.map(body=>`export default api=>{${body}};`)});}
const input={type:'input',text:'base',images:[{type:'image',data:'AA==',mimeType:'image/png'}],source:'rpc'};
add('input-none',input,[]);
add('input-continue',input,[`api.on('input',()=>({action:'continue'}));`]);
add('input-undefined',input,[`api.on('input',()=>undefined);`]);
add('input-transform-chain',input,[`api.on('input',e=>({action:'transform',text:e.text+' one'}));`,`api.on('input',e=>({action:'transform',text:e.text+' two'}));`]);
add('input-same-text-images',input,[`api.on('input',e=>({action:'transform',text:e.text,images:e.images}));`]);
add('input-images-replace',input,[`api.on('input',e=>({action:'transform',text:e.text,images:[]}));`]);
add('input-handled-stops',input,[`api.on('input',()=>({action:'handled',custom:'kept'}));`,`api.on('input',()=>{throw new Error('must not run')});`]);
add('input-throw-continues',input,[`api.on('input',()=>{throw new Error('fixture failure')});`,`api.on('input',e=>({action:'transform',text:e.text+' after'}));`]);
const call={type:'tool_call',toolName:'fixture',toolCallId:'call',input:{n:1}};
add('tool-call-none',call,[]);
add('tool-call-block',call,[`api.on('tool_call',()=>({block:true,reason:'fixture'}));`,`api.on('tool_call',()=>{throw new Error('must not run')});`]);
add('tool-call-last',call,[`api.on('tool_call',()=>({block:false,reason:'first'}));`,`api.on('tool_call',()=>{throw new Error('fixture failure')});api.on('tool_call',()=>({block:false,reason:'last'}));`]);
const toolResult={type:'tool_result',toolName:'fixture',toolCallId:'call',input:{},content:[{type:'text',text:'original'}],details:{custom:7},isError:true};
add('tool-result-none',toolResult,[]);
add('tool-result-chain',toolResult,[`api.on('tool_result',()=>({content:[{type:'text',text:'first'}]}));`,`api.on('tool_result',e=>({content:[...e.content,{type:'text',text:'second'}]}));`]);
add('tool-result-partial',toolResult,[`api.on('tool_result',()=>({isError:false}));`,`api.on('tool_result',e=>({details:{observed:e.isError,prior:e.details}}));`]);
add('tool-result-undefined-fields',toolResult,[`api.on('tool_result',()=>({content:undefined,details:undefined,isError:undefined}));`]);
add('tool-result-error-isolated',toolResult,[`api.on('tool_result',()=>{throw new Error('fixture failure')});`,`api.on('tool_result',()=>({details:null}));`]);
add('context-chain',{type:'context',messages:[{role:'user',content:'hello',timestamp:0}]},[`api.on('context',e=>({messages:[...e.messages,{role:'custom',payload:{unknown:3}}]}));`,`api.on('context',e=>({messages:e.messages.slice(1)}));`]);
add('context-empty',{type:'context',messages:[{role:'user',content:'hello',timestamp:0}]},[`api.on('context',()=>({messages:[]}));`]);
add('payload-null-chain',{type:'before_provider_request',payload:{initial:true}},[`api.on('before_provider_request',()=>null);`,`api.on('before_provider_request',e=>({observed:e.payload}));`]);
add('payload-undefined',{type:'before_provider_request',payload:{initial:true}},[`api.on('before_provider_request',()=>undefined);`]);
add('payload-error-continues',{type:'before_provider_request',payload:42},[`api.on('before_provider_request',()=>{throw new Error('fixture failure')});`,`api.on('before_provider_request',e=>[e.payload]);`]);
add('before-agent-chain',{type:'before_agent_start',prompt:'user',systemPrompt:'system'},[`api.on('before_agent_start',e=>({systemPrompt:e.systemPrompt+' first',message:{customType:'fixture',content:'one',display:false}}));`,`api.on('before_agent_start',e=>({systemPrompt:e.systemPrompt+' second',message:{customType:'other',content:'two',display:true,details:{unknown:4}}}));`]);
add('before-agent-empty-system',{type:'before_agent_start',prompt:'user',systemPrompt:'system'},[`api.on('before_agent_start',()=>({systemPrompt:''}));`]);
add('before-agent-none',{type:'before_agent_start',prompt:'user',systemPrompt:'system'},[]);
add('resources-annotated',{type:'resources_discover',cwd:'/fixture',reason:'reload'},[`api.on('resources_discover',()=>({skillPaths:['./skill'],promptPaths:['./prompt']}));`,`api.on('resources_discover',()=>({themePaths:['./theme'],skillPaths:['./skill']}));`]);
add('resources-none',{type:'resources_discover',cwd:'/fixture',reason:'reload'},[]);
add('bash-first',{type:'user_bash',command:'fixture',excludeFromContext:false},[`api.on('user_bash',()=>({result:{output:'first',exitCode:0,cancelled:false,truncated:false}}));`,`api.on('user_bash',()=>{throw new Error('must not run')});`]);
add('generic-discards-result',{type:'session_start'},[`api.on('session_start',()=>({ignored:true}));`]);
for(const type of ['session_before_switch','session_before_fork','session_before_compact','session_before_tree']){
 add(`${type}-cancel`,{type},[`api.on('${type}',()=>({cancel:true,custom:7}));`,`api.on('${type}',()=>{throw new Error('must not run')});`]);
 add(`${type}-last`,{type},[`api.on('${type}',()=>({cancel:false,custom:1}));`,`api.on('${type}',()=>({custom:2}));`]);
}
async function emit(r,e){switch(e.type){
case 'input':return r.emitInput(e.text,e.images,e.source);
case 'tool_call':return r.emitToolCall(e);
case 'tool_result':return r.emitToolResult(e);
case 'context':return r.emitContext(e.messages);
case 'before_provider_request':return r.emitBeforeProviderRequest(e.payload);
case 'before_agent_start':return r.emitBeforeAgentStart(e.prompt,e.images,e.systemPrompt);
case 'resources_discover':return r.emitResourcesDiscover(e.cwd,e.reason);
case 'user_bash':return r.emitUserBash(e);
default:return r.emit(e);
}}
const rows=[];
for(const spec of cases){
 const runtime=createExtensionRuntime(),extensions=[];
 for(const [i,source] of spec.sources.entries()){
  const factory=(await import(`data:text/javascript,${encodeURIComponent(source)}`)).default;
  extensions.push(await loadExtensionFromFactory(factory,'/fixture',createEventBus(),runtime,`/fixture/extension-${i}.ts`));
 }
 // These empty read-only ports are never accessed by these pure hook fixtures.
 const runner=new ExtensionRunner(extensions,runtime,'/fixture',Object.freeze({}),Object.freeze({}));
 const errors=[];runner.onError(e=>errors.push({extensionPath:e.extensionPath,event:e.event,error:e.error}));
 const output=packet(await emit(runner,structuredClone(spec.event)));
 rows.push({...spec,output,errors,hasHandlers:runner.hasHandlers(spec.event.type)});
}
const result={version:1,source_commit:'e30f0b4b238ee5618928bcd17b4571eb046e7038',rows};
const target=path.join(root,'fixtures/tools/extension-runner-oracle.json');
if(process.argv.includes('--capture')){assert(!fs.existsSync(target));fs.writeFileSync(target,JSON.stringify(result,null,2)+'\n');}
else assert.deepEqual(result,JSON.parse(fs.readFileSync(target,'utf8')));
console.log(JSON.stringify({status:'passed',extension_runner_cases:rows.length,owned_factories_only:true,network_actions:0,model_actions:0}));
