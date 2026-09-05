// Executes the unmodified pinned provider adapters on synthetic frames. No live HTTP.
import fs from 'node:fs';
import assert from 'node:assert/strict';
import {createRequire} from 'node:module';
import {fileURLToPath,pathToFileURL} from 'node:url';
import path from 'node:path';
await import('../offline_guard.mjs');
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../..');
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
 'anthropic-messages':['anthropic','streamAnthropic'],
 'openai-completions':['openai-completions','streamOpenAICompletions'],
 'mistral-conversations':['mistral','streamMistral'],
 'openai-responses':['openai-responses','streamOpenAIResponses'],
 'azure-openai-responses':['azure-openai-responses','streamAzureOpenAIResponses'],
 'openai-codex-responses':['openai-codex-responses','streamOpenAICodexResponses'],
 'google-generative-ai':['google','streamGoogle'],
 'google-gemini-cli':['google-gemini-cli','streamGoogleGeminiCli'],
 'google-vertex':['google-vertex','streamGoogleVertex'],
 'bedrock-converse-stream':['amazon-bedrock','streamBedrock'],
};
const output=[];
const previous=JSON.parse(fs.readFileSync(path.join(root,'fixtures/providers/oracle.json'),'utf8'));
const image={type:'image',mimeType:'image/png',data:'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aP2sAAAAASUVORK5CYII='};
for(const test of fixtures.cases) for(const scenario of ["roundtrip","cross-model","missing-result"]){
  active=test;requests=[];payload=undefined;
  const [file,symbol]=registry[test.api];
  const mod=await import(pathToFileURL(path.join(reference,`packages/ai/dist/providers/${file}.js`)));
  assert.equal(typeof mod[symbol],'function',symbol);
  const jwt='fixture.'+Buffer.from(JSON.stringify({'https://api.openai.com/auth':{chatgpt_account_id:'fixture-account'}})).toString('base64url')+'.fixture';
  const apiKey=test.api==='google-gemini-cli'?JSON.stringify({token:'fixture-token',projectId:'fixture-project'}):test.api==='openai-codex-responses'?jwt:'fixture-key';
  const model={id:'fixture',name:'Fixture',api:test.api,provider:test.api==='openai-codex-responses'?'openai-codex':test.api,baseUrl:'https://fixture.invalid/v1',reasoning:true,input:['text','image'],contextWindow:32768,maxTokens:4096,cost:{input:1,output:2,cacheRead:.1,cacheWrite:1.25}};
  const context={systemPrompt:'Fixture system',messages:[{role:'user',content:'Return a tool call or answer.',timestamp:0}],tools:[{name:'read',description:'Fixture read',parameters:{type:'object',properties:{path:{type:'string'}},required:['path']}}]};
  const prior=structuredClone(previous.groups.find(g=>g.api===test.api).result);
  context.messages.push(prior);
  if(scenario!=='missing-result') context.messages.push({role:'toolResult',toolCallId:prior.content.find(b=>b.type==='toolCall').id,toolName:'read',content:[{type:'text',text:'fixture file contents'},image],isError:false,timestamp:0});
  context.messages.push({role:'user',content:[{type:'text',text:'Continue from this image.'},image],timestamp:0});
  if(scenario==='cross-model') {prior.model='other-model';prior.provider='other-provider';prior.api='openai-completions';}
  const options={apiKey,transport:'sse',maxTokens:256,azureBaseUrl:'https://fixture.invalid',azureDeploymentName:'fixture',azureApiVersion:'2025-04-01-preview',onPayload:p=>{payload=structuredClone(p);}};
  const stream=mod[symbol](model,context,options);
  const result=await stream.result();
  assert.equal(result.stopReason,'toolUse',`${test.api}: ${result.errorMessage}`);
  assert.equal(result.content.filter(b=>b.type==='text').map(b=>b.text).join(''),test.expected_text,test.api);
  assert.equal(result.content.filter(b=>b.type==='thinking').map(b=>b.thinking).join(''),test.expected_thinking,test.api);
  const tool=result.content.find(b=>b.type==='toolCall');assert.equal(tool.name,'read');assert.deepEqual(tool.arguments,{path:'a.txt'});
  delete result.timestamp;
  assert(payload);assert(requests.length>0);
  output.push({api:test.api,scenario,model,context,options:{maxTokens:256},transport:requests});
  console.log(JSON.stringify({api:test.api,status:'passed',fake_requests:requests.length}));
}
const result=JSON.parse(JSON.stringify({version:1,source_commit:fixtures.source_commit,network_actions:0,model_actions:0,normalization:['Omitted undefined JS properties match JSON transport. Gemini CLI volatile requestId is replaced with <fixture-request-id>.'],groups:output},(key,value)=>key==='requestId'&&typeof value==='string'&&/^ag1-\d+-/.test(value)?'<fixture-request-id>':value));
const target=path.join(root,'fixtures/providers/requests.json');
if(process.argv.includes('--capture')){assert(!fs.existsSync(target),'reference oracle already frozen');fs.writeFileSync(target,JSON.stringify(result,null,2)+'\n');}
else {assert.deepEqual(result,JSON.parse(fs.readFileSync(target,'utf8')),'pinned provider oracle changed');}
console.log(JSON.stringify({status:'passed',provider_groups:output.length,network_actions:0,model_actions:0}));
