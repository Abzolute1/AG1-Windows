// Actual native detail forms against an owned in-process GitHub fixture only.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { runNativeFixture } from './p14_native_startup_smoke.mjs';
await runNativeFixture(process.argv[2], { prefix:'github-items-', githubFixture:true, async exercise({work,probe,wait,requests}) {
  const js=script=>probe({script});
  const click=label=>wait(()=>js(`(()=>{const b=[...document.querySelectorAll('button')].find(b=>b.getClientRects().length&&(b.getAttribute('aria-label')===${JSON.stringify(label)}||b.textContent.trim()===${JSON.stringify(label)}));if(!b||b.disabled)return false;b.focus();b.click();return true})()`),`click ${label}`);
  const fill=(label,value)=>js(`(()=>{const l=[...document.querySelectorAll('.github-item-actions label')].find(l=>l.firstChild.textContent===${JSON.stringify(label)});const e=l?.querySelector('input,textarea,select');if(!e)return false;if(e.tagName==='SELECT'){e.value=${JSON.stringify(value)};e.dispatchEvent(new Event('change',{bubbles:true}));}else{Object.getOwnPropertyDescriptor(e.tagName==='TEXTAREA'?HTMLTextAreaElement.prototype:HTMLInputElement.prototype,'value').set.call(e,${JSON.stringify(value)});e.dispatchEvent(new Event('input',{bubbles:true}));}return true})()`);
  const choose=value=>fill('Detail action',value);
  const calls=()=>{const p=path.join(work,'state/github-requests.jsonl');return fs.existsSync(p)?fs.readFileSync(p,'utf8').split('\n').slice(0,-1).filter(Boolean).map(JSON.parse):[];};
  const writes=()=>calls().filter(c=>c.operation.risk!=='read');
  const details=async()=>{await wait(()=>js(`(()=>{const s=[...document.querySelectorAll('summary')].find(s=>s.textContent==='Edit item and metadata');if(!s)return false;if(!s.parentElement.open)s.click();return s.parentElement.open})()`),'open item form');await wait(()=>js(`(()=>{const e=document.querySelector('.github-item-actions');if(!e||!e.getClientRects().length)return false;e.scrollIntoView({block:'start'});return true})()`),'visible item form');};
  await click('GitHub');await click('owned/fixture');await click('Issues');await click('Owned fixture issue');
  await click('Next comments');await wait(()=>js(`document.body.innerText.includes('Owned second comment page')`),'second native comment page');
  assert.equal(await js(`document.querySelector('.github-item script')!==null`),false);
  await click('Previous comments');await wait(()=>js(`document.body.innerText.includes('Comment page 1')&&!document.body.innerText.includes('Working with GitHub…')`),'first comment page settled');await details();await probe({screenshot:'github-item-form-wide.png'});
  const cases=[
    {action:'text',operation:'issues.update',body:{title:'Owned updated issue',body:'Literal <script>updated text</script>'},fill:async()=>{await fill('Updated title','Owned updated issue');await fill('Updated description','Literal <script>updated text</script>');}},
    {action:'state',operation:'issues.update',body:{state:'closed'},fill:async()=>fill('New item state','closed')},
    {action:'state',operation:'issues.update',body:{state:'open'},fill:async()=>fill('New item state','open')},
    {action:'labels',operation:'issues.update',body:{labels:['help wanted','literal,comma']},fill:async()=>fill('Replacement labels','help wanted\nliteral,comma')},
    {action:'assignees',operation:'issues.update',body:{assignees:['owned-fixture']},fill:async()=>fill('Replacement assignee logins','owned-fixture')},
    {action:'milestone',operation:'issues.update',body:{milestone:4},fill:async()=>fill('Milestone number','4')},
    {action:'milestone',operation:'issues.update',body:{milestone:null},fill:async()=>js(`document.querySelector('.github-item-actions input[type="checkbox"]').click();true`)},
    {action:'assignees',operation:'issues.update',body:{assignees:[]},fill:async()=>fill('Replacement assignee logins','')},
  ];
  for(const [index,c]of cases.entries()){
    await choose(c.action);await c.fill();const before=writes().length;
    await click('Review item change');await wait(()=>js(`!!document.querySelector('[aria-label="Exact native item change"]')`),`native ${c.action} preview`);
    assert.equal(writes().length,before);const request=await js(`JSON.parse(document.querySelector('[aria-label="Exact native item change"]').textContent)`);
    assert.equal(request.id,c.operation);assert.equal(request.repo,'owned/fixture');assert.deepEqual(request.body,c.body);
    if(index===3){await probe({size:[900,700]});await wait(()=>js('innerWidth===900'),'narrow item form');await js(`document.querySelector('.github-item-actions').scrollIntoView({block:'start'});true`);assert.ok(await js('document.documentElement.scrollWidth<=901'));await probe({screenshot:'github-item-label-preview-900.png'});}
    await click('Confirm item change');await wait(()=>writes().length===before+1,`owned ${c.action} publication`);try{await wait(()=>js(`document.querySelector('.github-item-actions').textContent.includes('GitHub accepted the request')`),'native result');}catch(e){fs.writeFileSync(path.join(work,'item-failure-details.json'),JSON.stringify(await js(`({form:document.querySelector('.github-item-actions')?.outerHTML})`),null,2));throw e;}
    assert.deepEqual(writes().at(-1).operation.body,c.body);
  }
  await click('Refresh item details');await wait(()=>js(`document.querySelector('.github-item h3')?.textContent.includes('Owned updated issue')`),'fresh changed issue detail');
  assert.ok(await js(`document.querySelector('.github-item').innerText.includes('help wanted, literal,comma')`));assert.equal(await js(`!!document.querySelector('.github-item script')`),false);
  await details();await probe({screenshot:'github-item-updated-900.png'});
  await click('Back to list');
  const filter=value=>js(`(()=>{const e=document.querySelector('[aria-label="GitHub item state filter"]');if(!e||e.disabled)return false;e.value=${JSON.stringify(value)};e.dispatchEvent(new Event('change',{bubbles:true}));return true})()`);
  await wait(()=>filter('closed'),'select closed items');await wait(()=>js(`document.body.innerText.includes('No matching issues on this page.')`),'closed list excludes reopened issue');
  await wait(()=>filter('open'),'select open items');await wait(()=>js(`!![...document.querySelectorAll('.github-items button')].find(b=>b.textContent.includes('Owned updated issue'))`),'open list includes reopened issue');
  await click('Pull requests');await click('Owned fixture pull request');await details();await choose('reviewers');await fill('Reviewer logins','owned-fixture');await fill('Reviewer team slugs','core');
  const before=writes().length;await click('Review item change');await wait(()=>js(`!!document.querySelector('[aria-label="Exact native item change"]')`),'native reviewers preview');assert.equal(writes().length,before);await probe({screenshot:'github-item-reviewers-900.png'});
  await click('Confirm item change');await wait(()=>writes().length===before+1,'owned reviewer request');await wait(()=>js(`document.querySelector('.github-item-actions').textContent.includes('GitHub accepted the request')`),'reviewer request result');
  assert.equal(writes().at(-1).operation.id,'pulls.reviewers');assert.deepEqual(writes().at(-1).operation.body,{reviewers:['owned-fixture'],team_reviewers:['core']});
  await click('Refresh item details');await wait(()=>js(`document.querySelector('.github-item')?.innerText.includes('Requested reviewers: owned-fixture · Teams: core')`),'fresh reviewer metadata');
  assert.equal(writes().length,9);assert.equal(requests.length,0);
  return {promptsSubmitted:0,providerRequests:requests,githubCalls:calls(),liveGitHub:false,checks:['native issue title/body update and close/reopen','exact labels and assignees replacement with explicit clearing','milestone assignment and explicit removal','PR reviewer login/team request','all nine changes require distinct exact native previews and confirmation; preview never publishes','fresh reads show fixture-persisted metadata','independent bounded comment pages and literal repository text','wide/narrow forms and source previews; no provider requests, live GitHub or paid CI']};
}});
