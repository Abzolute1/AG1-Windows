// Owned account-only + model PR review through the real shared native panel.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { runNativeFixture } from './p14_native_startup_smoke.mjs';
let generations=0;const errors=[];
function handleRequest(request,response){let body='';request.on('data',p=>{body+=p;if(body.length>4*1024*1024)request.destroy();});request.on('end',()=>{try{
  const data=JSON.parse(body);assert.ok(generations<2);assert.ok(data.tools.some(t=>t.function?.name==='github'));const first=generations++===0;
  if(!first){const result=data.messages.find(m=>m.role==='tool'&&m.tool_call_id==='owned-pr-read');assert.ok(JSON.stringify(result).includes('Owned fixture pull request'));}
  const delta=first?{tool_calls:[{index:0,id:'owned-pr-read',type:'function',function:{name:'github',arguments:JSON.stringify({operation:'pulls.get',repo:'owned/fixture',parameters:{number:7}})}}]}:{content:'The owned PR is available for explicit file and check review; nothing was merged.'};
  response.writeHead(200,{'Content-Type':'text/event-stream',Connection:'close'});response.end(`data: ${JSON.stringify({choices:[{delta,finish_reason:first?'tool_calls':'stop'}],usage:{prompt_tokens:100,completion_tokens:40}})}\n\ndata: [DONE]\n\n`);
}catch(e){errors.push(String(e));response.writeHead(400);response.end('{}');}});}
await runNativeFixture(process.argv[2],{prefix:'github-pr-review-',githubFixture:true,handleRequest,async exercise({work,probe,wait,requests}){
  const js=script=>probe({script});
  const click=label=>wait(()=>js(`(()=>{const b=[...document.querySelectorAll('button')].find(b=>b.getClientRects().length&&(b.getAttribute('aria-label')===${JSON.stringify(label)}||b.textContent.trim()===${JSON.stringify(label)}));if(!b||b.disabled)return false;b.focus();b.click();return true})()`),`click ${label}`);
  const fill=(selector,value)=>js(`(()=>{const e=document.querySelector(${JSON.stringify(selector)});if(!e)return false;Object.getOwnPropertyDescriptor(e.tagName==='TEXTAREA'?HTMLTextAreaElement.prototype:HTMLInputElement.prototype,'value').set.call(e,${JSON.stringify(value)});e.dispatchEvent(new Event('input',{bubbles:true}));return true})()`);
  const selectFile=name=>wait(()=>js(`(()=>{const b=[...document.querySelectorAll('[aria-label="Pull request files"] button')].find(b=>b.querySelector('span')?.textContent===${JSON.stringify(name)});if(!b)return false;b.focus();b.click();return true})()`),`select PR file ${name}`);
  const openPR=async()=>{await click('GitHub');await click('owned/fixture');await click('Owned fixture pull request');await click('Review changes');await wait(()=>js(`!!document.querySelector('[aria-label="Pull request files"] button')`),'shared native PR review');};
  await openPR();
  assert.equal(await js(`document.querySelector('[aria-label="Review views"]').textContent`),'GitHub PR');
  await selectFile('reviewed.txt');assert.equal(await js(`document.querySelectorAll('[aria-label="Selected PR file diff"] pre > span').length`),300);
  assert.equal(await js(`getComputedStyle(document.querySelector('.pr-diff-gutter')).display`),'inline');
  assert.equal(await js(`document.querySelectorAll('[aria-label="Selected PR file diff"] script').length`),0);
  await probe({screenshot:'github-pr-review-wide.png'});await probe({size:[900,700]});await wait(()=>js('innerWidth===900'),'narrow PR panel');
  await js(`document.querySelector('[aria-label="Selected PR file diff"]').scrollIntoView({block:'start'});true`);await probe({screenshot:'github-pr-review-patch-900.png'});
  await click('Next patch page');await wait(()=>js(`document.querySelector('[aria-label="Selected PR file diff"]').textContent.includes('Patch page 2')`),'bounded patch paging');
  await click('Next PR files');await wait(()=>js(`document.querySelector('[aria-label="Pull request files"]')?.textContent.includes('second-page.bin')`),'second exact file page');await selectFile('second-page.bin');
  assert.ok(await js(`document.querySelector('[aria-label="Selected PR file diff"]').textContent.includes('This is not an empty diff')`));
  await click('Next PR checks');await wait(()=>js(`document.querySelector('[aria-label="Pull request review"]')?.textContent.includes('Owned second check')`),'independent checks page');
  await js(`document.querySelector('[aria-label="Pull request review"] h3:last-of-type').scrollIntoView({block:'center'});true`);await probe({screenshot:'github-pr-review-checks-900.png'});
  await click('Close review panel');await wait(()=>js(`document.activeElement.textContent==='Review changes'`),'focus restored to PR opener');
  await click('Back to chat');const project=path.join(work,'project');fs.mkdirSync(project,{mode:0o700});await js(`document.dispatchEvent(new KeyboardEvent('keydown',{key:'o',ctrlKey:true,bubbles:true}));true`);await wait(()=>js(`!!document.querySelector('input[aria-label="Project folder"]')`),'owned project dialog');await fill('input[aria-label="Project folder"]',project);await click('Open project');await wait(()=>js(`!document.querySelector('input[aria-label="Project folder"]')`),'owned project open');
  await fill('textarea[aria-label="Message AG1"]','Preserve this unsent draft.');await openPR();await click('Close review panel');await click('Back to chat');assert.equal(await js(`document.querySelector('textarea[aria-label="Message AG1"]').value`),'Preserve this unsent draft.');
  await fill('textarea[aria-label="Message AG1"]','Read owned/fixture pull request 7. Do not post a review or merge anything.');await click('Send message');await wait(()=>js(`!!document.querySelector('.approval-review')`),'model exact GitHub read approval');await click('Allow this operation');await wait(()=>generations===2,'model native result');
  await wait(()=>js(`document.body.innerText.includes('nothing was merged.')`),'model completion');await click('Review PR files and checks');await wait(()=>js(`!!document.querySelector('[aria-label="Pull request files"] button')`),'model card shared review');await selectFile('reviewed.txt');await probe({screenshot:'github-pr-review-model-900.png'});
  await probe({size:[1440,900]});await wait(()=>js('innerWidth===1440'),'wide panel');await click('GitHub');await click('Disconnect');assert.equal(await js(`!!document.querySelector('[aria-label="Pull request review"]')`),false);
  const calls=fs.readFileSync(path.join(work,'state/github-requests.jsonl'),'utf8').trim().split('\n').map(JSON.parse);assert.ok(calls.every(c=>c.operation.risk==='read'));assert.ok(calls.some(c=>c.operation.endpoint.includes('/files?page=2')));assert.ok(calls.some(c=>c.operation.endpoint.includes('/check-runs?page=2')));assert.equal(requests.length,2);assert.deepEqual(errors,[]);
  return{promptsSubmitted:1,providerRequests:requests,githubCalls:calls,liveGitHub:false,checks:['account-only PR review requires no local project','shared panel shows bounded literal and numbered patch rows','file and check pages retain exact head/base independently','missing binary patch is not represented as an empty diff','close restores focus and unsent chat draft survives review','model read waits for native approval and result card opens the same PR panel','disconnect discards open remote review','read-only owned API fixture; no reviews, merges, CI or live GitHub']};
}});
