// Owned native graph mutation forms and fixture-persisted refreshes.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import {runNativeFixture} from './p14_native_startup_smoke.mjs';
await runNativeFixture(process.argv[2],{prefix:'graph-edit-',githubFixture:true,async exercise({work,probe,wait,requests}){
  const js=script=>probe({script});
  const click=label=>wait(()=>js(`(()=>{const b=[...document.querySelectorAll('button')].find(b=>b.getClientRects().length&&(b.getAttribute('aria-label')===${JSON.stringify(label)}||b.textContent.trim()===${JSON.stringify(label)}));if(!b||b.disabled)return false;b.click();return true})()`),`click ${label}`);
  const select=(selector,value)=>js(`(()=>{const e=document.querySelector(${JSON.stringify(selector)});if(!e||e.disabled)return false;e.value=${JSON.stringify(value)};e.dispatchEvent(new Event('change',{bubbles:true}));return true})()`);
  const fill=(label,value)=>js(`(()=>{const l=[...document.querySelectorAll('.github-graph-actions label')].find(e=>e.firstChild?.textContent===${JSON.stringify(label)});const e=l?.querySelector('input,textarea');if(!e||e.disabled)return false;Object.getOwnPropertyDescriptor(e.tagName==='TEXTAREA'?HTMLTextAreaElement.prototype:HTMLInputElement.prototype,'value').set.call(e,${JSON.stringify(value)});e.dispatchEvent(new Event('input',{bubbles:true}));return true})()`);
  const row=(name,inside=false)=>wait(()=>js(`(()=>{const b=[...document.querySelectorAll(${JSON.stringify(inside?'.github-graph-detail .github-items button':'.github-browser .github-items button')})].find(b=>b.querySelector('strong')?.textContent===${JSON.stringify(name)});if(!b)return false;b.click();return true})()`),`row ${name}`);
  const text=value=>wait(()=>js(`document.querySelector('.github-graph-detail')?.textContent.includes(${JSON.stringify(value)})`),`text ${value}`);
  const calls=()=>{const p=path.join(work,'state/github-requests.jsonl');return fs.existsSync(p)?fs.readFileSync(p,'utf8').split('\n').slice(0,-1).filter(Boolean).map(JSON.parse):[];};
  const writes=()=>calls().filter(c=>c.operation.risk!=='read');
  const form=()=>wait(()=>js(`(()=>{const s=[...document.querySelectorAll('.github-graph-detail summary')].find(s=>s.textContent==='Project or discussion changes');if(!s)return false;if(!s.parentElement.open)s.click();return !!document.querySelector('.github-graph-actions')})()`),'open graph form');
  const change=async(action,fields={})=>{
    await form();assert.equal(await select('[aria-label="Graph action"]',action),true);for(const[label,value]of Object.entries(fields))assert.equal(await fill(label,value),true);
    const before=writes().length;await click('Review graph change');await wait(()=>js(`!!document.querySelector('[aria-label="Exact native graph change"]')`),'native graph preview');assert.equal(writes().length,before);
    const ack=await js(`(()=>{const e=document.querySelector('.github-graph-actions input[type="checkbox"]');if(!e)return false;return true})()`);
    if(ack){assert.equal(await js(`([...document.querySelectorAll('.github-graph-actions button')].find(b=>b.textContent==='Confirm graph change')).disabled`),true);await js(`document.querySelector('.github-graph-actions input[type="checkbox"]').click();true`);}
    await probe({size:[900,700]});await wait(()=>js('innerWidth===900'),'narrow mutation form');await js(`document.querySelector('.github-graph-actions').scrollIntoView({block:'start'});true`);assert.ok(await js('document.documentElement.scrollWidth<=901'));await probe({screenshot:`graph-change-${before+1}-900.png`});
    await click('Confirm graph change');await wait(()=>writes().length===before+1,'one fixture mutation');await wait(()=>js(`document.querySelector('.github-graph-actions')?.textContent.includes('GitHub responded.')`),'native change response');
  };
  await click('GitHub');await click('owned/fixture');await js(`(()=>{const s=[...document.querySelectorAll('summary')].find(s=>s.textContent==='Code, workflows, releases, projects and notifications');s.click();return true})()`);
  await select('[aria-label="GitHub repository view"]','projects.user_list');await click('Load view');await row('Owned project');await click('Inspect project items and fields');await text('Owned graph detail');
  await change('edit',{'Graph title':'Updated owned project','Graph text':'New owned readme <script>literal</script>'});await click('Refresh graph view');await text('Updated owned project');await text('New owned readme');
  await change('state');await click('Refresh graph view');await text('Closed');
  await change('draft',{'Graph title':'Owned draft from form','Graph text':'Owned draft bytes'});await click('Project items');await row('Owned draft from form',true);await text('Owned draft bytes');await row('Owned task',true);await click('Inspect item field values');await row('Done',true);
  await change('set',{'Field value':'Owned changed field'});await click('Refresh graph view');await row('Status',true);await text('Owned changed field');await change('clear');await click('Refresh graph view');await text('Item field values');assert.equal(await js(`!![...document.querySelectorAll('.github-graph-detail .github-items strong')].find(e=>e.textContent==='Status')`),false);
  await change('archive');await click('Refresh graph view');await form();await text('Unarchive this item.');
  await select('[aria-label="GitHub repository view"]','discussions.list');await click('Load view');await row('Owned discussion');await click('Inspect discussion and replies');await text('First comment');
  await change('edit',{'Graph title':'Updated owned discussion','Graph text':'Updated discussion body'});await click('Refresh graph view');await text('Updated owned discussion');await text('Updated discussion body');
  await change('state');await click('Refresh graph view');await text('Closed');await change('state');await click('Refresh graph view');await text('Open');
  await change('comment',{'Graph text':'New owned discussion comment'});await click('Refresh graph view');await text('New owned discussion comment');
  await row('owned-commenter',true);await change('edit',{'Graph text':'Edited owned comment'});await click('Refresh graph view');await text('Edited owned comment');await row('owned-commenter',true);
  await change('answer');await click('Refresh graph view');await row('owned-commenter',true);await form();await select('[aria-label="Graph action"]','answer');await text('Unmark this comment as the answer.');await change('answer');await click('Refresh graph view');await row('owned-commenter',true);await form();await select('[aria-label="Graph action"]','answer');await text('Mark this comment as the answer.');
  await change('reply',{'Graph text':'New owned reply'});await click('Inspect comment replies');await text('New owned reply');
  assert.equal(writes().length,14);assert.equal(requests.length,0);assert.ok(writes().every(c=>c.operation.repo===null));assert.equal(await js(`!!document.querySelector('.github-graph-detail script')`),false);
  return{promptsSubmitted:0,providerRequests:requests,githubCalls:calls(),liveGitHub:false,checks:['fourteen graph changes each require a fresh exact native confirmation','project edits, close and draft persist across fresh reads','field set/clear and archive refresh from fixture state','discussion edit, close/reopen, comment/edit/reply and answer/unanswer refresh from fixture state','account-wide target IDs and consequential acknowledgement','literal text and narrow-screen form/preview','zero live GitHub or provider traffic']};
}});
