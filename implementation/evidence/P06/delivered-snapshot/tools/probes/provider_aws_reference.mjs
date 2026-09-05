import fs from 'node:fs';
import assert from 'node:assert/strict';
import {createRequire} from 'node:module';
import {fileURLToPath} from 'node:url';
import path from 'node:path';
import {Readable} from 'node:stream';
await import('../offline_guard.mjs');
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../..');
for(const key of Object.keys(process.env))if(/API_KEY|TOKEN|SECRET|CREDENTIAL|^AWS_|PROXY/i.test(key))delete process.env[key];
const require=createRequire(path.join(root,'reference/ag1-executable/packages/ai/package.json'));
const {SignatureV4}=require('@smithy/signature-v4');
const {Hash}=require('@smithy/hash-node');
const {BedrockRuntimeClient,ConverseStreamCommand}=require('@aws-sdk/client-bedrock-runtime');
const credentials={accessKeyId:'AKIDFIXTURE',secretAccessKey:'fixture-secret',sessionToken:'fixture-session-token'};
const signer=new SignatureV4({service:'bedrock',region:'us-east-1',credentials,sha256:Hash.bind(null,'sha256')});
const groups=[];
for(const test of [
 {path:'/model/fixture/converse-stream',query:{},body:'{"messages":[]}'},
 {path:'/model/arn%3Aaws%3Abedrock%3Aus-east-1%3A123%3Amodel%2Ffixture/converse-stream',query:{z:'last',a:['two words','+'],empty:''},body:'{"text":"moon 🌙"}'},
 {path:'/double//path/./tail/../end/',query:{'unicode🌙':'a/b?c'},body:''},
]){
 const input={protocol:'https:',hostname:'bedrock-runtime.us-east-1.amazonaws.com',method:'POST',headers:{host:'bedrock-runtime.us-east-1.amazonaws.com','content-type':'application/json','x-fixture':'  multiple   spaces  ','user-agent':'ag1-rust/0.1.0'},...test};
 const signed=await signer.sign(input,{signingDate:new Date('2026-09-05T00:00:00Z')});
 groups.push({input,headers:signed.headers,date:'20260905T000000Z',region:'us-east-1'});
}
const requests=JSON.parse(fs.readFileSync(path.join(root,'fixtures/providers/requests.json'),'utf8')).groups.filter(g=>g.api==='bedrock-converse-stream');
const serialized=[];
for(const test of requests){
 let captured;
 const client=new BedrockRuntimeClient({credentials,region:'us-east-1',maxAttempts:1,requestHandler:{handle:async(request)=>{
  captured={path:request.path,method:request.method,body:JSON.parse(request.body),content_type:request.headers['content-type']};
  return {response:{statusCode:200,headers:{'content-type':'application/vnd.amazon.eventstream'},body:Readable.from([fs.readFileSync(path.join(root,'fixtures/providers/bedrock-events.bin'))])}};
 }}});
 const input=structuredClone(test.transport[0].body);
 function revive(value){if(!value||typeof value!=='object')return;for(const [k,v]of Object.entries(value)){if(k==='bytes'){value[k]=Uint8Array.from(Object.keys(v).sort((a,b)=>Number(a)-Number(b)).map(k=>v[k]));}else revive(v);}}
 revive(input);
 const result=await client.send(new ConverseStreamCommand(input));let count=0;for await(const frame of result.stream){assert(frame);count++;}assert(count>0);
 serialized.push({scenario:test.scenario,...captured});client.destroy();
}
const out={version:1,source_commit:'e30f0b4b238ee5618928bcd17b4571eb046e7038',credentials,groups,serialized,network_actions:0};
const target=path.join(root,'fixtures/providers/aws.json');
if(process.argv.includes('--capture')){assert(!fs.existsSync(target));fs.writeFileSync(target,JSON.stringify(out,null,2)+'\n');}else assert.deepEqual(out,JSON.parse(fs.readFileSync(target,'utf8')));
console.log(JSON.stringify({status:'passed',signatures:groups.length,http_serializations:serialized.length,network_actions:0}));
