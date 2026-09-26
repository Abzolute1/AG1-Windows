// Native WebKit + native host + its own fixed-response child. No inference.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import vm from 'node:vm';
import { runNativeFixture } from './p14_native_startup_smoke.mjs';
assert.equal(process.argv.length, 3, 'Supply only an isolated visual-test candidate');

await runNativeFixture(process.argv[2], { prefix: 'owned-', ownedRuntime: true,
  async exercise({ work, probe, wait, requests }) {
    const project = path.join(work, 'project'); fs.mkdirSync(project, { mode: 0o700 });
    fs.writeFileSync(path.join(project, 'sentinel.txt'), 'No model or tool execution\n');
    const events = () => {
      const file = path.join(work, 'runtime-events.jsonl');
      if (!fs.existsSync(file)) return [];
      const text = fs.readFileSync(file, 'utf8');
      return text.slice(0, text.lastIndexOf('\n') + 1).split('\n').filter(Boolean).map(line => JSON.parse(line));
    };
    const js = script => { new vm.Script(script); return probe({ script }); };
    const click = label => wait(() => js(`(()=>{const b=[...document.querySelectorAll('button')].find(b=>b.getAttribute('aria-label')===${JSON.stringify(label)}||b.getAttribute('title')===${JSON.stringify(label)}||b.textContent.trim()===${JSON.stringify(label)});if(!b||b.disabled)return false;b.click();return true})()`), `click ${label}`);
    const fill = async (selector, value) => assert.equal(await js(`(()=>{const e=document.querySelector(${JSON.stringify(selector)});if(!e)return false;Object.getOwnPropertyDescriptor(e.tagName==='TEXTAREA'?HTMLTextAreaElement.prototype:HTMLInputElement.prototype,'value').set.call(e,${JSON.stringify(value)});e.dispatchEvent(new Event('input',{bubbles:true}));return true})()`), true);
    const composer = 'textarea[aria-label="Message AG1"]';
    const native = async () => (await probe({ native_view: true })).view;
    const settled = () => wait(async () => { const s = (await native()).snapshot; return !s.active_operation && !s.pending.length && !['running', 'streaming', 'awaiting_input'].includes(s.status); }, 'native completion after owned shutdown');
    const checks = [], cases = [];
    await click('Back to chat');
    await js(`document.dispatchEvent(new KeyboardEvent('keydown',{key:'o',ctrlKey:true,bubbles:true}));true`);
    await wait(() => js(`!!document.querySelector('input[aria-label="Project folder"]')`), 'project dialog');
    await fill('input[aria-label="Project folder"]', project); await click('Open project');
    await wait(() => js(`!document.querySelector('input[aria-label="Project folder"]')`), 'project opened');
    assert.equal((await native()).snapshot.workspace_path, project);

    for (const mode of ['streaming', 'before-output']) {
      // Each scenario gets a clean conversation; recovery remains in that same scope.
      if (mode === 'before-output') await click('New chat');
      const prompt = `P14 OWNED ${mode}`;
      const before = events().length;
      await fill(composer, prompt); await click('Send message');
      await wait(() => events().slice(before).some(e => e.event === 'request' && e.prompt === prompt), `${mode} actually dispatched`);
      const request = events().slice(before).find(e => e.event === 'request' && e.prompt === prompt);
      if (mode === 'streaming') await wait(() => js(`document.body.innerText.includes('OWNED_STREAM_PULSE')`), 'stream visible before Stop');
      else assert.equal(await js(`document.body.innerText.includes('OWNED_STREAM_PULSE')||document.body.innerText.includes('OWNED_RECOVERY_COMPLETE')`), false);
      const active = (await native()).snapshot;
      assert.ok(active.active_operation);
      await fill(composer, `Draft survives ${mode}`);
      await probe({ screenshot: `${mode}-before-stop.png` });
      const start = Date.now(); await click('Stop generation');
      await wait(() => events().slice(before).some(e => e.event === 'cancel_reaped' && e.pid === request.pid && e.waited === true), 'owned HTTP server killed and reaped');
      await settled();
      const cancelMs = Date.now() - start;
      assert.equal((await native()).snapshot.scope.session, active.scope.session);
      assert.equal(await js(`document.querySelector(${JSON.stringify(composer)}).value`), `Draft survives ${mode}`);
      await wait(async () => !await js(`document.body.innerText.includes('Stopping response')`), 'renderer receives settled native snapshot');
      assert.deepEqual(await js(`[...document.querySelectorAll('[role=alert]')].map(e=>e.textContent).filter(Boolean)`), []);
      const recoveryStart = events().length;
      await fill(composer, 'P14 OWNED recovery'); await click('Send message');
      await wait(() => events().slice(recoveryStart).some(e => e.event === 'terminal'), 'fresh request completes on replacement process');
      await settled();
      await wait(() => js(`document.body.innerText.includes('OWNED_RECOVERY_COMPLETE')`), 'recovery reply visible');
      const recovery = events().slice(recoveryStart).find(e => e.event === 'request');
      assert.notEqual(recovery.pid, request.pid, 'Recovery must use a fresh owned server');
      assert.equal((await native()).snapshot.scope.session, active.scope.session);
      cases.push({ mode, cancelledPid: request.pid, recoveryPid: recovery.pid, cancelMs, scope: active.scope, operation: active.active_operation });
      checks.push(`${mode}: dispatched Stop reaps actual server, native settles, draft survives, same-chat recovery succeeds on fresh process`);
      await probe({ screenshot: `${mode}-recovered.png` });
    }
    assert.deepEqual(requests, [], 'Unowned HTTP sink must never be used');
    const providerRequests = events().filter(e => e.event === 'request');
    assert.equal(providerRequests.length, 4);
    assert.equal(events().filter(e => e.event === 'cancel_reaped').length, 2);
    assert.equal(fs.readFileSync(path.join(project, 'sentinel.txt'), 'utf8'), 'No model or tool execution\n');
    return { promptsSubmitted: 4, providerRequests, checks, cases,
      limits: ['fixed responses, not real-model cancellation or GPU verification', 'diagnostic DOM control, not physical input', 'no approval expiry, installation or duration run'] };
  },
});
