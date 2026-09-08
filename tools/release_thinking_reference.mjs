// Executes the unmodified pinned provider adapters on synthetic frames. No live HTTP.
import fs from 'node:fs';
import assert from 'node:assert/strict';
import {createRequire} from 'node:module';
import {fileURLToPath,pathToFileURL} from 'node:url';
import path from 'node:path';
await import('./offline_guard.mjs');
const root=process.cwd();
const reference=path.join(root,'reference/ag1-executable');
for(const key of Object.keys(process.env)) if(/API_KEY|TOKEN|SECRET|CREDENTIAL|^AWS_|^GOOGLE_|^ANTHROPIC_|PROXY/i.test(key)) delete process.env[key];
process.env.AWS_BEDROCK_SKIP_AUTH='1';
const fixtures=JSON.parse(fs.readFileSync(path.join(root,'fixtures/providers/wire.json'),'utf8'));
let active,requests=[],payload;
globalThis.fetch=async(input,options={})=>{
  assert(active,'unscoped fixture HTTP');
  const request=input instanceof Request?input:new Request(input,options);
  const body=await request.clone().text();
  requests.push({url:request.url,method:request.method,header_names:[...request.headers.keys()].sort(),body:body?JSON.parse(body):null});
  const data=active.frames.map(frame=>`${frame.type?'event: '+frame.type+'\n':''}data: ${JSON.stringify(frame)}\n\n`).join('');
  const bytes=new TextEncoder().encode(data);let offset=0;
  return new Response(new ReadableStream({pull(controller){if(offset===bytes.length){controller.close();return;}const end=Math.min(bytes.length,offset+7);controller.enqueue(bytes.slice(offset,end));offset=end;}}),{status:200,headers:{'content-type':'text/event-stream'}});
};
const require=createRequire(path.join(reference,'packages/ai/package.json'));
// The pinned completion adapter explicitly uses undici.fetch with its own Agent.
require('undici').fetch=globalThis.fetch;
const bedrock=require('@aws-sdk/client-bedrock-runtime');
const {EventStreamCodec}=require('@smithy/eventstream-codec');
const codec=new EventStreamCodec(v=>new TextDecoder().decode(v),v=>new TextEncoder().encode(v));
const awsCase=fixtures.cases.find(c=>c.api==='bedrock-converse-stream');
const awsBytes=Buffer.concat(awsCase.frames.map(frame=>{
  const [name,body]=Object.entries(frame)[0];
  return Buffer.from(codec.encode({headers:{':message-type':{type:'string',value:'event'},':event-type':{type:'string',value:name},':content-type':{type:'string',value:'application/json'}},body:new TextEncoder().encode(JSON.stringify(body))}));
}));
const awsFixture=path.join(root,'fixtures/providers/bedrock-events.bin');
if(process.argv.includes('--capture-aws')){assert(!fs.existsSync(awsFixture));fs.writeFileSync(awsFixture,awsBytes);}
else {assert.deepEqual(awsBytes,fs.readFileSync(awsFixture),'AWS SDK binary fixture changed');}
bedrock.BedrockRuntimeClient.prototype.send=async function(command){
  assert.equal(active.api,'bedrock-converse-stream');
  requests.push({sdk:'BedrockRuntimeClient.send',body:command.input});
  return {stream:(async function*(){for(const frame of active.frames)yield structuredClone(frame);})()};
};
const registry={
 'anthropic-messages':['anthropic','streamSimpleAnthropic'],
 'openai-completions':['openai-completions','streamSimpleOpenAICompletions'],
 'mistral-conversations':['mistral','streamSimpleMistral'],
 'openai-responses':['openai-responses','streamSimpleOpenAIResponses'],
 'azure-openai-responses':['azure-openai-responses','streamSimpleAzureOpenAIResponses'],
 'openai-codex-responses':['openai-codex-responses','streamSimpleOpenAICodexResponses'],
 'google-generative-ai':['google','streamSimpleGoogle'],
 'google-gemini-cli':['google-gemini-cli','streamSimpleGoogleGeminiCli'],
 'google-vertex':['google-vertex','streamSimpleGoogleVertex'],
 'bedrock-converse-stream':['amazon-bedrock','streamSimpleBedrock'],
};
const output=[];
const previous=JSON.parse(fs.readFileSync(path.join(root,'fixtures/providers/oracle.json'),'utf8'));
const image={type:'image',mimeType:'image/png',data:'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aP2sAAAAASUVORK5CYII='};
const variants={
 'anthropic-messages':['claude-sonnet-4-5','claude-opus-4-6','claude-sonnet-4-6'],
 'openai-completions':['Qwen3.6-35B-A3B-UD-Q4_K_XL','gpt-5.2','fixture'],
 'mistral-conversations':['magistral-small'],
 'openai-responses':['gpt-5.2','fixture'],
 'azure-openai-responses':['gpt-5.2','fixture'],
 'openai-codex-responses':['gpt-5.2','gpt-5.1-codex-mini'],
 'google-generative-ai':['gemini-2.5-pro','gemini-2.5-flash','gemini-3.1-pro','gemini-3-flash','fixture'],
 'google-gemini-cli':['gemini-2.5-pro','gemini-3.1-pro','gemini-3-flash'],
 'google-vertex':['gemini-2.5-pro','gemini-2.5-flash','gemini-3.1-pro','gemini-3-flash'],
 'bedrock-converse-stream':['anthropic.claude-sonnet-4-5-v1:0','anthropic.claude-opus-4-6-v1','fixture'],
};
for(const test of fixtures.cases) for(const id of variants[test.api]) for(const scenario of ['off','minimal','low','medium','high','xhigh','max']){
  active=test;requests=[];payload=undefined;
  const [file,symbol]=registry[test.api];
  const mod=await import(pathToFileURL(path.join(reference,`packages/ai/dist/providers/${file}.js`)));
  assert.equal(typeof mod[symbol],'function',symbol);
  const jwt='fixture.'+Buffer.from(JSON.stringify({'https://api.openai.com/auth':{chatgpt_account_id:'fixture-account'}})).toString('base64url')+'.fixture';
  const apiKey=test.api==='google-gemini-cli'?JSON.stringify({token:'fixture-token',projectId:'fixture-project'}):test.api==='openai-codex-responses'?jwt:'fixture-key';
  const model={id,name:'Fixture',api:test.api,provider:test.api==='openai-codex-responses'?'openai-codex':id.startsWith('Qwen')?'llamacpp':test.api,baseUrl:'https://fixture.invalid/v1',reasoning:true,input:['text','image'],contextWindow:32768,maxTokens:4096,cost:{input:1,output:2,cacheRead:.1,cacheWrite:1.25}};
  if(id.startsWith('Qwen'))model.compat={thinkingFormat:'qwen-chat-template',supportsDeveloperRole:true};
  const context={systemPrompt:'Fixture system',messages:[{role:'user',content:'Return a tool call or answer.',timestamp:0}],tools:[{name:'read',description:'Fixture read',parameters:{type:'object',properties:{path:{type:'string'}},required:['path']}}]};
  const requestOptions={maxTokens:256,temperature:.2,sessionId:'fixture-session',cacheRetention:'none'};
  if(scenario!=='off')requestOptions.reasoning=scenario;
  const options={apiKey,transport:'sse',maxTokens:256,azureBaseUrl:'https://fixture.invalid',azureDeploymentName:'fixture',azureApiVersion:'2025-04-01-preview',...requestOptions,onPayload:p=>{payload=structuredClone(p);}};
  const stream=mod[symbol](model,context,options);
  const result=await stream.result();
  assert.equal(result.stopReason,'toolUse',`${test.api}: ${result.errorMessage}`);
  assert.equal(result.content.filter(b=>b.type==='text').map(b=>b.text).join(''),test.expected_text,test.api);
  assert.equal(result.content.filter(b=>b.type==='thinking').map(b=>b.thinking).join(''),test.expected_thinking,test.api);
  const tool=result.content.find(b=>b.type==='toolCall');assert.equal(tool.name,'read');assert.deepEqual(tool.arguments,{path:'a.txt'});
  delete result.timestamp;
  assert(payload);assert(requests.length>0);
  output.push({api:test.api,scenario,model,context,options:requestOptions,transport:requests});
  console.log(JSON.stringify({api:test.api,status:'passed',fake_requests:requests.length}));
}
const result=JSON.parse(JSON.stringify({version:1,source_commit:fixtures.source_commit,network_actions:0,model_actions:0,normalization:['Omitted undefined JS properties match JSON transport. Gemini CLI volatile requestId is replaced with <fixture-request-id>.'],groups:output},(key,value)=>key==='requestId'&&typeof value==='string'&&/^ag1-\d+-/.test(value)?'<fixture-request-id>':value));
const target=path.join(root,'fixtures/release/session-thinking.json');
assert.deepEqual(result,JSON.parse(fs.readFileSync(target,'utf8')),'Pinned simple-stream thinking oracle changed');
console.log(JSON.stringify({status:'passed',provider_groups:output.length,network_actions:0,model_actions:0}));
