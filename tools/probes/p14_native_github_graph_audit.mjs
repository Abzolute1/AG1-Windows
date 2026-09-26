// Owned native Projects/Discussions reads only; no live GitHub or inference.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { runNativeFixture } from './p14_native_startup_smoke.mjs';
await runNativeFixture(process.argv[2],{prefix:'github-graph-',githubFixture:true,async exercise({work,probe,wait,requests}){
  const js=script=>probe({script});
  const click=label=>wait(()=>js(`(()=>{const b=[...document.querySelectorAll('button')].find(b=>b.getClientRects().length&&(b.getAttribute('aria-label')===${JSON.stringify(label)}||b.textContent.trim()===${JSON.stringify(label)}));if(!b||b.disabled)return false;b.focus();b.click();return true})()`),`click ${label}`);
  const selectView=value=>js(`(()=>{const e=document.querySelector('[aria-label="GitHub repository view"]');if(!e||e.disabled)return false;e.value=${JSON.stringify(value)};e.dispatchEvent(new Event('change',{bubbles:true}));return true})()`);
  const row=(name,inside=false)=>wait(()=>js(`(()=>{const b=[...document.querySelectorAll(${JSON.stringify(inside?'.github-graph-detail .github-items button':'.github-browser .github-items button')})].find(b=>b.querySelector('strong')?.textContent===${JSON.stringify(name)});if(!b||b.disabled)return false;b.focus();b.click();return true})()`),`select ${name}`);
  const text=needle=>wait(()=>js(`document.querySelector('.github-graph-detail')?.textContent.includes(${JSON.stringify(needle)})`),`graph ${needle}`);
  const shot=async name=>{await js(`document.querySelector('.github-graph-detail')?.scrollIntoView({block:'start'});true`);await probe({screenshot:name});};
  await click('GitHub');await click('owned/fixture');
  await wait(()=>js(`(()=>{const s=[...document.querySelectorAll('summary')].find(s=>s.textContent==='Code, workflows, releases, projects and notifications');if(!s)return false;if(!s.parentElement.open)s.click();return true})()`),'open repository browser');
  await selectView('projects.user_list');await click('Load view');await row('Owned project');await click('Inspect project items and fields');await text('Owned graph detail');
  assert.equal(await js(`!!document.querySelector('.github-graph-detail script')`),false);await shot('github-project-overview-wide.png');
  await click('Project fields');await row('Status',true);await text('O_done');await click('Next graph page');await row('Second field',true);await text('Second field');
  await probe({size:[900,700]});await wait(()=>js('innerWidth===900'),'narrow graph detail');assert.ok(await js('document.documentElement.scrollWidth<=901'));await shot('github-project-fields-900.png');
  await click('Project items');await row('Owned task',true);await click('Inspect item field values');await row('Done',true);await text('V_status');await click('Next graph page');await row('Second value',true);await shot('github-project-values-900.png');
  for (const [name,kind] of [['People','users'],['Labels','labels'],['Linked PRs','pull_requests'],['Reviewers','reviewers']]) {
    await row(name,true);await click('Browse field values');await row(`First ${kind} value`,true);await click('Next graph page');await row(`Second ${kind} value`,true);
    assert.ok(await js('document.documentElement.scrollWidth<=901'));await shot(`github-project-${kind}-900.png`);await click('Back to item field values');await text('V_status');
  }
  await selectView('discussions.list');await click('Load view');await row('Owned discussion');await click('Inspect discussion and replies');await text('First comment');
  await click('Next graph page');await text('Second comment');await click('Previous graph page');await text('First comment');await row('owned-commenter',true);await click('Inspect comment replies');await text('First reply');await click('Next graph page');await text('Second reply');await shot('github-discussion-replies-900.png');
  await click('Discussion categories');await row('Ideas',true);await click('Next graph page');await row('Second category',true);await shot('github-discussion-categories-900.png');
  assert.equal(await js(`!!document.querySelector('.github-graph-detail script')`),false);
  const calls=fs.readFileSync(path.join(work,'state/github-requests.jsonl'),'utf8').split('\n').slice(0,-1).filter(Boolean).map(JSON.parse);
  assert.ok(calls.every(c=>c.operation.risk==='read'));assert.equal(requests.length,0);
  for(const [operation,cursor]of [['projects.fields','fields-cursor'],['projects.item_values','fieldValues-cursor'],['discussions.get','comments-cursor'],['discussions.replies','replies-cursor'],['discussions.categories','categories-cursor']])assert.ok(calls.some(c=>c.operation.id===operation&&c.operation.body.variables.after===cursor),`${operation} cursor`);
  for (const kind of ['users','labels','pull_requests','reviewers']) { const rows=calls.filter(c=>c.operation.id===`projects.field_${kind}`);assert.equal(rows.length,2);assert.ok(rows.every(c=>c.operation.repo===null&&c.operation.body.variables.id==='I_owned'&&typeof c.operation.body.variables.fieldId==='string'&&typeof c.operation.body.variables.fieldName==='string'));assert.equal(rows[1].operation.body.variables.after,`${kind}-cursor`); }
  assert.ok(calls.filter(c=>c.operation.id==='projects.item_values').every(c=>c.operation.body.variables.id==='I_owned'&&c.operation.repo===null));
  assert.ok(calls.filter(c=>c.operation.id==='discussions.replies').every(c=>c.operation.body.variables.id==='C_owned'&&c.operation.repo===null));
  return{promptsSubmitted:0,providerRequests:requests,githubCalls:calls,liveGitHub:false,checks:['account-wide project overview uses an explicit native node ID','independent project fields/items/item-value reads with exact cursor variables','repository-bound discussion detail/comment pages','explicit account-wide comment node for separately paged replies','independently paginated discussion categories','bounded literal source and text, wide/narrow layout','all native read-only operations, zero prompts, provider requests or mutations']};
}});
