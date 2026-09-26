// Owned offline registry -> contained local setup -> scoped native permission.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import {execFileSync} from 'node:child_process';
import {runNativeFixture} from './p14_native_startup_smoke.mjs';
let fixture, directory, modelStep=0, currentWork;
const interactive=process.env.MCP_NATIVE_INPUT==='1';
const delay=ms=>new Promise(r=>setTimeout(r,ms));
function matches(value) {
 if(typeof value==='string'){try{return matches(JSON.parse(value))}catch{return null}}
 if(!value||typeof value!=='object')return null;
 if(Array.isArray(value.matches))return value.matches;
 for(const v of Object.values(value)){const result=matches(v);if(result)return result}return null;
}
function model(request,response){
 let data='';request.on('data',b=>{data+=b;if(data.length>1048576)request.destroy()});request.on('end',()=>{
  try{
   const body=JSON.parse(data);fs.appendFileSync(path.join(currentWork,'model-requests.jsonl'),JSON.stringify(body)+'\n');
   const step=modelStep++;let name,args,message;
   if(step===0)message='Ready for owned MCP setup.';
   else if(step===1){name='mcpDiscover';args={action:'search',kind:'tool',query:'owned_read',limit:1}}
   else if(step===2){const entry=matches(body)?.[0];assert.ok(entry,'native discovery match');name='mcpDiscover';args={action:'select',entries:[{connectionId:entry.connectionId,kind:'tool',key:entry.key}]}}
   else if(step===3){name=body.tools.find(t=>t.function.name.startsWith('mcp_')).function.name;args={}}
   else message='Owned MCP workflow complete.';
   const delta=name?{role:'assistant',tool_calls:[{index:0,id:'owned-call-'+step,type:'function',function:{name,arguments:JSON.stringify(args)}}]}:{role:'assistant',content:message};
   response.writeHead(200,{'Content-Type':'text/event-stream'});
   response.end('data: '+JSON.stringify({id:'owned-completion',object:'chat.completion.chunk',model:'startup-fixture',choices:[{index:0,delta,finish_reason:null}]})+'\n\ndata: '+JSON.stringify({id:'owned-completion',object:'chat.completion.chunk',choices:[{index:0,delta:{},finish_reason:name?'tool_calls':'stop'}],usage:{prompt_tokens:100,completion_tokens:20,total_tokens:120}})+'\n\ndata: [DONE]\n\n');
  }catch(e){fs.writeFileSync(path.join(currentWork,'model-error.txt'),String(e));response.writeHead(500);response.end('{}')}
 });
}
await runNativeFixture(process.argv[2], {
 prefix:interactive?'f-':'i-',
 ...(interactive?{handleRequest:model}:{}),
 async beforeStart({work}) {
  currentWork=work;
  if(interactive){const p=path.join(work,'host.json'),c=JSON.parse(fs.readFileSync(p));c.max_generations=6;fs.writeFileSync(p,JSON.stringify(c))}
  fixture=path.join(work,'peer');fs.mkdirSync(fixture,{mode:0o700});
  fs.writeFileSync(path.join(fixture,'server.py'),`import sys,json\nfor line in sys.stdin:\n r=json.loads(line)\n if 'id' not in r: continue\n m=r.get('method')\n if m=='server/discover': v={'resultType':'complete','supportedVersions':['2026-07-28'],'capabilities':{'tools':{}},'ttlMs':0,'cacheScope':'private'}\n elif m=='tools/list': v={'tools':[{'name':'owned_read','description':'Owned read. Literal <script>untrusted</script>','inputSchema':{'type':'object','properties':{},'additionalProperties':False}}]}\n else: raise Exception('No tool calls permitted')\n print(json.dumps({'jsonrpc':'2.0','id':r['id'],'result':v}),flush=True)\n`);
  if(interactive) fs.writeFileSync(path.join(fixture,'server.py'),`import sys,json
for line in sys.stdin:
 r=json.loads(line)
 if 'id' not in r: continue
 m=r.get('method')
 if m=='server/discover': v={'resultType':'complete','supportedVersions':['2026-07-28'],'capabilities':{'tools':{}},'ttlMs':0,'cacheScope':'private'}
 elif m=='tools/list': v={'tools':[{'name':'owned_read','description':'Owned read. Literal <script>untrusted</script>','inputSchema':{'type':'object','properties':{},'additionalProperties':False}}]}
 elif m=='tools/call':
  if 'inputResponses' not in r['params']: v={'resultType':'input_required','requestState':'owned-private-state','inputRequests':{'review':{'method':'elicitation/create','params':{'mode':'form','message':'Include the owned example image?','requestedSchema':{'type':'object','properties':{'confirm':{'type':'boolean','title':'Include image'}},'required':['confirm']}}}}}
  else:
   assert r['params']['requestState']=='owned-private-state' and r['params']['inputResponses']['review']['content']['confirm']==True
   v={'resultType':'complete','content':[{'type':'text','text':'Owned image result.'},{'type':'image','mimeType':'image/png','data':'iVBORw0KGgoAAAANSUhEUgAAAAgAAAAICAIAAABLbSncAAAAEElEQVR4nGNgaGjAjoaWBADiMEABWMSWggAAAABJRU5ErkJggg=='},{'type':'resource_link','name':'Owned reference','uri':'fixture://owned/reference'}]}
 else: raise Exception('Unexpected method')
 print(json.dumps({'jsonrpc':'2.0','id':r['id'],'result':v}),flush=True)
`);
  directory=path.join(work,'owned-mcp-directory.json');
  fs.writeFileSync(directory,JSON.stringify({server:{name:'org.example/owned',title:'Owned integration server',version:'1.0.0',description:'Literal <script>registry metadata</script>',packages:[{registryType:'pypi',identifier:'owned-fixture',version:'1.0.0',transport:{type:'stdio'}}]},_meta:{'io.modelcontextprotocol.registry/official':{status:'active',isLatest:true}}}),{mode:0o600});
 },
 async exercise({work,probe,wait,requests}) {
  const js=script=>probe({script});
  const click=label=>wait(()=>js(`(()=>{const e=[...document.querySelectorAll('button')].find(e=>e.getAttribute('aria-label')===${JSON.stringify(label)}||e.textContent.trim()===${JSON.stringify(label)});if(!e||e.disabled)return false;e.click();return true})()`),`click ${label}`);
  const text=value=>wait(()=>js(`document.body.innerText.includes(${JSON.stringify(value)})`),value);
  const fill=async(label,value)=>assert.equal(await js(`(()=>{const l=[...document.querySelectorAll('label')].find(e=>e.textContent.startsWith(${JSON.stringify(label)}));const e=l?.querySelector('input,textarea,select');if(!e)return false;Object.getOwnPropertyDescriptor(e.tagName==='SELECT'?HTMLSelectElement.prototype:e.tagName==='TEXTAREA'?HTMLTextAreaElement.prototype:HTMLInputElement.prototype,'value').set.call(e,${JSON.stringify(value)});e.dispatchEvent(new Event(e.tagName==='SELECT'?'change':'input',{bubbles:true}));return true})()`),true);
  const ack=selector=>js(`(()=>{document.querySelector(${JSON.stringify(selector)}).click();return true})()`);
  const send=async message=>{await js(`(()=>{const e=document.querySelector('[aria-label="Message AG1"]');Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,'value').set.call(e,${JSON.stringify(message)});e.dispatchEvent(new Event('input',{bubbles:true}));return true})()`);await click('Send message')};
  await click('Back to chat');if(interactive){await send('Prepare the owned MCP workflow.');await text('Ready for owned MCP setup.')}await click('MCP');await text('No servers installed.');
  await click('Discover');await text('Search to fetch listings.');
  await fill('Search public servers','owned');await click('Search');await text('Owned integration server');
  await js(`document.querySelector('[aria-label="Public MCP listings"] button').click();true`);await text('Configure existing local installation');
  await probe({screenshot:'discover-wide.png'});await probe({size:[900,600]});await wait(()=>js('innerWidth===900'),'narrow viewport');await delay(200);await probe({screenshot:'discover-narrow.png'});
  await click('Configure existing local installation');
  await fill('Installed executable','/usr/bin/python3');await fill('Installed arguments',JSON.stringify(['-I',path.join(fixture,'server.py')]));await fill('Reviewed working directory',fs.realpathSync(fixture));
  await click('Review local setup');await text('Review server setup');await probe({screenshot:'setup-review.png'});
  await ack('[aria-label="Review discovered server"] input[type=checkbox]');await click('Save discovered server inactive');await text('Unavailable here');
  await click('Enable here');await click('Connect');await text('Review connection');await probe({screenshot:'contained-connection-review.png'});
  await ack('[aria-label="Review MCP connection"] input[type=checkbox]');await click('Connect reviewed target');
  // This harness's outer user namespace intentionally leaves host root unmapped.
  // Production correctly refuses its root-owned-launcher check; record that
  // failure, then explicitly choose host-account mode for this owned peer only.
  await text('failed · This chat');await probe({screenshot:'nested-containment-refused.png'});
  await click('Edit configuration');await fill('Local process access','hostAccount');await click('Save inactive');await text('Unavailable here');
  await click('Enable here');await click('Connect');await text('Review connection');await ack('[aria-label="Review MCP connection"] input[type=checkbox]');await click('Connect reviewed target');await text('ready · This chat');
  await click('Tools');await text('owned_read');
  await js(`document.querySelector('.mcp-capabilities button').click();true`);await text('Review policy for owned_read');
  await fill('Reviewed effect','read');await fill('Approval behavior','allow');await fill('Argument scope','allArguments');await click('Review rule');await text('Confirm the native rule review');
  await js(`document.querySelector('[aria-label="Review MCP rule"]').scrollIntoView({block:'start'});true`);await delay(200);await probe({screenshot:'permission-review.png'});await ack('[aria-label="Review MCP rule"] input[type=checkbox]');await click('Save reviewed rules');await text('1 capability rule set saved');
  if(interactive){
   await click('Connect');await text('Review connection');await ack('[aria-label="Review MCP connection"] input[type=checkbox]');await click('Connect reviewed target');await text('ready · This chat');
   await click('Back to chat');await send('Check the owned MCP server and show its example image.');
   const helper=path.resolve('tools/probes/mcp_form_input.py');
   const input=(action,...args)=>JSON.parse(execFileSync('/usr/bin/python3',[helper,work,action,'form',...args],{encoding:'utf8',timeout:10000}));
   await wait(()=>input('status').dialog,'native form');input('screenshot','input-form.png');
   input('answer');input('screenshot','input-answer.png');console.log(JSON.stringify({answeredOwnedForm:true,work}),true);
   const until=Date.now()+100000;while(input('status').dialog&&Date.now()<until)await delay(200);
   await text('Owned MCP workflow complete.');await wait(()=>js(`!!document.querySelector('[aria-label="Preview MCP owned_read preview 1"]')`),'native result image');
   await probe({screenshot:'mcp-result-image.png'});
   await js(`document.querySelector('.work-toggle').click();true`);await click('MCP · owned_read');await text('Inspect original MCP response');
   await js(`document.querySelector('.mcp-result').scrollIntoView({block:'start'});true`);await delay(200);await probe({screenshot:'mcp-result-source.png'});
   await click('Inspect original MCP response');await text('Text page starting at byte');await probe({screenshot:'mcp-result-original.png'});
   assert.ok(!fs.readFileSync(path.join(work,'model-requests.jsonl'),'utf8').includes('owned-private-state'));
   await click('MCP');await text('Owned integration server');await js(`document.querySelector('[aria-label="Installed MCP servers"] button').click();true`);await text('Scope policy');
  }
  await fill('Scope policy','disableWrites');await click('Save scope policy');await text('Disable writes');
  await probe({size:[1440,900]});await js(`document.querySelector('.mcp-permissions').scrollIntoView({block:'start'});true`);await probe({screenshot:'permissions-wide.png'});
  await click('Revoke rules for owned_read');await text('No capability rules saved');
  await click('Disable here');await click('Remove server');await click('Confirm removal');await text('No servers installed.');
  if(!interactive)assert.deepEqual(requests,[]);assert.equal(await js(`document.querySelector('.mcp-discover script')===null`),true);
  return {scope:'general-chat',registry:'owned offline response reader',localAccess:'explicit hostAccount after nested-namespace containment refusal; actual confinement covered by owned native contract',workflow:['discover','review existing installation','save inactive','enable','connect','inspect','review scoped read grant','disable writes','revoke','disable','remove'],providerRequests:requests,promptsSubmitted:interactive?2:0};
 }
});
