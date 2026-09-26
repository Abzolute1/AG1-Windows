// Owned fixture reviews/merge; never sends a real GitHub request.
import assert from 'node:assert/strict';import fs from 'node:fs';import path from 'node:path';
import {runNativeFixture} from './p14_native_startup_smoke.mjs';
await runNativeFixture(process.argv[2],{prefix:'pull-',githubFixture:true,async exercise({work,probe,wait,requests}){
 const js=script=>probe({script});
 const click=label=>wait(()=>js(`(()=>{const b=[...document.querySelectorAll('button')].find(b=>b.getClientRects().length&&(b.getAttribute('aria-label')===${JSON.stringify(label)}||b.textContent.trim()===${JSON.stringify(label)}));if(!b||b.disabled)return false;b.focus();b.click();return true})()`),`click ${label}`);
 const calls=()=>{const p=path.join(work,'state/github-requests.jsonl');return fs.existsSync(p)?fs.readFileSync(p,'utf8').trim().split('\n').filter(Boolean).map(JSON.parse):[];};const writes=()=>calls().filter(c=>c.operation.risk!=='read');
 await click('GitHub');await click('owned/fixture');await click('Owned fixture pull request');
 for(const event of ['COMMENT','APPROVE','REQUEST_CHANGES']){
  await click('Submit review');await wait(()=>js(`(()=>{const f=document.querySelector('.github-compose'),s=f?.querySelector('select'),e=f?.querySelector('textarea');if(!s||!e)return false;s.value=${JSON.stringify(event)};s.dispatchEvent(new Event('change',{bubbles:true}));Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,'value').set.call(e,'Owned exact commit review');e.dispatchEvent(new Event('input',{bubbles:true}));return true})()`),'fill review');
  const before=writes().length;await click('Review');await wait(()=>js(`!!document.querySelector('[aria-label="Exact native GitHub request"]')`),'exact review preview');assert.equal(writes().length,before);
  const body=await js(`JSON.parse(document.querySelector('[aria-label="Exact native GitHub request"]').textContent)`);assert.deepEqual(body,{body:'Owned exact commit review',event,commit_id:'a'.repeat(40)});
  if(event==='COMMENT'){await probe({size:[900,700]});await js(`document.querySelector('.github-compose').scrollIntoView({block:'start'});true`);await probe({screenshot:'pull-review-confirmation-900.png'});}
  await click('Publish to GitHub');await wait(()=>writes().length===before+1,'owned review sent');await wait(()=>js(`!document.querySelector('.github-compose')&&!!document.querySelector('.github-item')`),'validated review receipt');assert.equal(writes().at(-1).operation.id,'pulls.review');
 }
 await click('Merge…');await click('Review');await wait(()=>js(`!!document.querySelector('[aria-label="Exact native GitHub request"]')`),'merge preview');assert.equal(writes().length,3);await click('Close GitHub form');assert.equal(writes().length,3);
 await click('Merge…');await click('Review');await wait(()=>js(`!!document.querySelector('[aria-label="Exact native GitHub request"]')`),'fresh merge preview');assert.deepEqual(await js(`JSON.parse(document.querySelector('[aria-label="Exact native GitHub request"]').textContent)`),{sha:'a'.repeat(40),merge_method:'squash'});
 await js(`document.querySelector('.github-compose').scrollIntoView({block:'start'});true`);await probe({screenshot:'pull-merge-confirmation-900.png'});await click('Apply change');await wait(()=>writes().length===4,'owned merge sent');await wait(()=>js(`!document.querySelector('.github-compose')&&!!document.querySelector('.github-item')`),'validated merge receipt');assert.equal(writes().at(-1).operation.id,'pulls.merge');assert.equal(requests.length,0);
 return{promptsSubmitted:0,providerRequests:requests,githubCalls:calls(),liveGitHub:false,checks:['comment, approval and change-request reviews bind the loaded commit','native previews send no writes','dismissed merge is not sent; fresh confirmation binds head SHA and squash method','native typed receipt validation before successful form completion','four owned fixture writes; no real reviews, merges, CI or model traffic']};
}});
