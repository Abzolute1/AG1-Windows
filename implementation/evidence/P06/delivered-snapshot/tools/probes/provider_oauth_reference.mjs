// Frozen refresh requests/results from the unchanged SDK; fake credentials and HTTP only.
import fs from 'node:fs';
import assert from 'node:assert/strict';
import path from 'node:path';
import {fileURLToPath,pathToFileURL} from 'node:url';
await import('../offline_guard.mjs');
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../..');
for(const key of Object.keys(process.env)) if(/API_KEY|TOKEN|SECRET|CREDENTIAL|^AWS_|^GOOGLE_|PROXY/i.test(key)) delete process.env[key];
Date.now=()=>1000000;
let response,requests=[];
globalThis.fetch=async(input,options)=>{
 const req=new Request(input,options),text=await req.text();
 requests.push({url:req.url,method:req.method,headers:Object.fromEntries(req.headers),body:text?(req.headers.get('content-type')?.includes('json')?JSON.parse(text):text):null});
 return new Response(JSON.stringify(response),{status:200,headers:{'content-type':'application/json'}});
};
const jwt='fixture.'+Buffer.from(JSON.stringify({'https://api.openai.com/auth':{chatgpt_account_id:'fixture-account'}})).toString('base64url')+'.fixture';
const cases=[['anthropic','refreshAnthropicToken'],['openai-codex','refreshOpenAICodexToken'],['github-copilot','refreshGitHubCopilotToken']];
const groups=[];
for(const [file,symbol] of cases){
 const mod=await import(pathToFileURL(path.join(root,`reference/ag1-executable/packages/ai/dist/utils/oauth/${file}.js`)));
 for(const enterprise of file==='github-copilot'?[undefined,'enterprise.fixture.invalid']:[undefined]){
  response=file==='github-copilot'?{token:'fixture-access',expires_at:4600}:{access_token:file==='openai-codex'?jwt:'fixture-access',refresh_token:'fixture-rotated',expires_in:3600};
  requests=[];
  const result=await mod[symbol]('fixture-refresh',enterprise);
  assert.equal(requests.length,1);
  groups.push({service:file,old:{type:'oauth',access:'fixture-old',refresh:'fixture-refresh',expires:0,...(enterprise?{enterpriseUrl:enterprise}:{})},response,result,request:requests[0]});
 }
}
const out={version:1,source_commit:'e30f0b4b238ee5618928bcd17b4571eb046e7038',network_actions:0,groups};
const target=path.join(root,'fixtures/providers/oauth.json');
if(process.argv.includes('--capture')){assert(!fs.existsSync(target));fs.writeFileSync(target,JSON.stringify(out,null,2)+'\n');}
// Match the JSON persistence boundary: undefined optional fields are omitted.
else assert.deepEqual(JSON.parse(JSON.stringify(out)),JSON.parse(fs.readFileSync(target,'utf8')));
console.log(JSON.stringify({status:'passed',refresh_groups:groups.length,network_actions:0}));
