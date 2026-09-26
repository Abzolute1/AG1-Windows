// Actual native host + WebKit; bounded scripted HTTP, no real model or tools.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import vm from 'node:vm';
import { runNativeFixture } from './p14_native_startup_smoke.mjs';

assert.equal(process.argv.length, 3, 'Supply only the isolated candidate directory');
const prompts = { a: 'P14 CHAT A streaming', b: 'P14 CHAT B reply', recovery: 'P14 RECOVERY reply' };
const exchanges = [], failures = [], active = new Map();
const finish = new Map();
const event = (text, finish = null) => `data: ${JSON.stringify({ choices: [{ delta: { content: text }, finish_reason: finish }] })}\n\n`;
function handleRequest(request, response) {
  let body = '', size = 0;
  request.on('data', chunk => { size += chunk.length; if (size > 4 * 1024 * 1024) request.destroy(); else body += chunk; });
  request.on('error', error => failures.push(String(error)));
  request.on('end', () => {
    try {
      assert.equal(request.method, 'POST'); assert.equal(request.url, '/chat/completions');
      const payload = JSON.parse(body);
      assert.equal(payload.model, 'startup-fixture'); assert.equal(payload.stream, true);
      const users = payload.messages.filter(m => m.role === 'user').map(m => typeof m.content === 'string' ? m.content : m.content.map(b => b.text || '').join(''));
      const last = users.findLast(text => Object.values(prompts).includes(text));
      const key = Object.keys(prompts).find(key => prompts[key] === last);
      assert.ok(key, 'Unexpected fixture prompt'); assert.ok(!active.has(key), 'Unexpected retry');
      if (key !== 'a') assert.ok(!JSON.stringify(payload.messages).includes(prompts.a), 'Chat A history/state leaked to Chat B');
      const record = { key, users, pulses: 0, closed: false, ended: false, timeout: false };
      active.set(key, record); exchanges.push(record);
      let pulse, deadline;
      response.on('close', () => { record.closed = true; clearInterval(pulse); clearTimeout(deadline); });
      response.on('error', error => failures.push(String(error)));
      if (key === 'a') {
        deadline = setTimeout(() => { record.timeout = true; failures.push(`${key} fixture exceeded 45 seconds`); response.destroy(); }, 45000);
        response.writeHead(200, { 'Content-Type': 'text/event-stream', Connection: 'close' });
        const send = () => { if (!response.destroyed) response.write(event(`A_STREAM_${record.pulses++} `)); };
        send(); pulse = setInterval(send, 150);
        finish.set(key, () => { clearInterval(pulse); clearTimeout(deadline); record.ended = true; response.end(event('A_FINISHED', 'stop') + 'data: [DONE]\n\n'); });
      } else {
        response.writeHead(200, { 'Content-Type': 'text/event-stream', Connection: 'close' });
        record.ended = true;
        response.end(event(key === 'b' ? 'B_ONLY_REPLY' : 'RECOVERY_COMPLETE', 'stop') + 'data: [DONE]\n\n');
      }
    } catch (error) {
      failures.push(String(error));
      if (!response.headersSent) response.writeHead(400, { 'Content-Type': 'application/json' });
      response.end(JSON.stringify({ error: { message: 'Controlled fixture rejected unexpected request' } }));
    }
  });
}

await runNativeFixture(process.argv[2], { prefix: 'lifecycle-', handleRequest,
  async exercise({ work, probe, wait, requests }) {
    const project = path.join(work, 'project'); fs.mkdirSync(project, { mode: 0o700 });
    fs.writeFileSync(path.join(project, 'sentinel.txt'), 'No file tool is permitted\n');
    const js = async script => { new vm.Script(script); return probe({ script }); };
    const click = label => wait(() => js(`(()=>{const b=[...document.querySelectorAll('button')].find(b=>b.getAttribute('aria-label')===${JSON.stringify(label)}||b.getAttribute('title')===${JSON.stringify(label)}||b.textContent.trim()===${JSON.stringify(label)});if(!b||b.disabled)return false;b.click();return true})()`), `click ${label}`);
    const fill = async (selector, value) => assert.equal(await js(`(()=>{const e=document.querySelector(${JSON.stringify(selector)});if(!e)return false;Object.getOwnPropertyDescriptor(e.tagName==='TEXTAREA'?HTMLTextAreaElement.prototype:HTMLInputElement.prototype,'value').set.call(e,${JSON.stringify(value)});e.dispatchEvent(new Event('input',{bubbles:true}));return true})()`), true);
    const composer = 'textarea[aria-label="Message AG1"]';
    const draft = () => js(`document.querySelector(${JSON.stringify(composer)})?.value`);
    const native = async () => (await probe({ native_view: true })).view;
    const settled = () => wait(async () => { const v = await native(); return !v.snapshot.active_operation && !v.snapshot.pending.length && !['running', 'streaming', 'awaiting_input'].includes(v.snapshot.status); }, 'native completion');
    const send = async key => { await fill(composer, prompts[key]); await click('Send message'); await wait(() => active.has(key), `${key} request received`); };
    const checks = [], identities = {}, timings = {};
    try {
      await click('Back to chat');
      await js(`document.dispatchEvent(new KeyboardEvent('keydown',{key:'o',ctrlKey:true,bubbles:true}));true`);
      await wait(() => js(`!!document.querySelector('input[aria-label="Project folder"]')`), 'project dialog');
      await fill('input[aria-label="Project folder"]', project); await click('Open project');
      await wait(() => js(`!document.querySelector('input[aria-label="Project folder"]')`), 'project opened');
      assert.equal((await native()).snapshot.workspace_path, project);
      await send('a'); await wait(() => js(`document.body.innerText.includes('A_STREAM_')`), 'chat A streaming visible');
      identities.a = (await native()).snapshot;
      await fill(composer, 'Unsent draft belonging to A');
      await click('New chat');
      await wait(async () => (await native()).snapshot.scope.session !== identities.a.scope.session, 'fresh chat B');
      identities.b = (await native()).snapshot;
      await wait(async () => await draft() === '', 'new chat clears composer');
      assert.equal(identities.b.workspace_path, project);
      assert.equal(await js(`document.body.innerText.includes('A_STREAM_')`), false);
      assert.equal(active.get('a').closed, false);

      await fill(composer, prompts.b); await click('Send message');
      await wait(() => js(`document.body.innerText.includes('Queued · waiting for model')`), 'B honestly queued behind local slot');
      identities.queued = (await native()).snapshot;
      assert.equal(active.has('b'), false, 'Queued cancellation must not dispatch a provider request');
      assert.equal(active.get('a').closed, false, 'Background A must survive navigation');
      const pulses = active.get('a').pulses;
      await wait(() => active.get('a').pulses > pulses + 1, 'A continues streaming in background');
      await fill(composer, 'Unsent draft belonging to B');
      await probe({ screenshot: 'chat-b-background-a.png' });
      const queuedCancelBegan = Date.now(); await click('Stop generation'); await settled();
      timings.queuedCancelMs = Date.now() - queuedCancelBegan;
      assert.equal(active.has('b'), false);
      assert.equal(active.get('a').closed, false, 'Cancelling queued B must not cancel streaming A');
      assert.equal((await native()).snapshot.scope.session, identities.b.scope.session);
      assert.equal(await draft(), 'Unsent draft belonging to B');
      await click(prompts.a);
      await wait(async () => (await native()).snapshot.scope.session === identities.a.scope.session && await draft() === 'Unsent draft belonging to A', 'A draft restored');
      assert.equal((await native()).snapshot.active_operation, identities.a.active_operation);
      await click('Settings'); await wait(() => js(`document.body.innerText.includes('Appearance')`), 'settings during streaming');
      assert.equal(active.get('a').closed, false);
      await click('Back to chat');
      checks.push('new chat stays in project; A keeps streaming while B queues; cancelling queued B never dispatches it or cancels A; drafts stay isolated');

      finish.get('a')(); await settled();
      await wait(() => active.get('a').closed, 'A terminal response closes');
      assert.equal(await draft(), 'Unsent draft belonging to A');
      await probe({ screenshot: 'chat-a-completed.png' });
      await click(prompts.b);
      await wait(async () => (await native()).snapshot.scope.session === identities.b.scope.session && await draft() === 'Unsent draft belonging to B', 'B draft restored');
      await send('recovery'); await settled();
      await wait(() => js(`document.body.innerText.includes('RECOVERY_COMPLETE')`), 'fresh B request completes after queued cancellation');
      assert.equal((await native()).snapshot.scope.session, identities.b.scope.session);
      checks.push('settings leaves A alive; terminal A releases local slot; fresh B request succeeds after queued cancellation');

      assert.equal(requests.length, 2); assert.deepEqual(failures, []);
      assert.ok(exchanges.every(r => !r.timeout && r.closed));
      assert.equal(fs.readFileSync(path.join(project, 'sentinel.txt'), 'utf8'), 'No file tool is permitted\n');
      await probe({ screenshot: 'chat-recovery-complete.png' });
      return { promptsSubmitted: 3, provider: 'scripted loopback only; no model', checks, timings,
        identities: Object.fromEntries(Object.entries(identities).map(([key, s]) => [key, { scope: s.scope, operation: s.active_operation }])),
        limits: ['fixture has no owned runtime cancellation proof; recovery after dispatched cancellation not claimed', 'diagnostic DOM control, not physical input', 'no live inference or GPU scheduling measurement', 'no native wall-clock approval expiry test'] };
    } finally {
      fs.writeFileSync(path.join(work, 'provider-lifecycle.json'), JSON.stringify({ exchanges, failures, checks, timings }, null, 2) + '\n');
    }
  },
});
