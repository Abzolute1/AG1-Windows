// Real native host + WebKit, scripted loopback provider, disposable files only.
// DOM-driven diagnostic broker: not physical-keyboard or live-model acceptance.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { runNativeFixture } from './p14_native_startup_smoke.mjs';

const cases = ['allow', 'deny', 'close', 'source-conflict'];
const content = 'Reviewed fixture content <script>literal only</script>\n';
const longContent = content + 'Long approval review fixture line.\n'.repeat(200);
const baseline = 'Original fixture bytes\n';
const external = 'External change during review\n';
const providerErrors = [], exchanges = [], results = [];
let current, stage = 0, promptsSubmitted = 0;

function handleRequest(request, response) {
  let body = '', bytes = 0;
  request.on('data', chunk => {
    bytes += chunk.length;
    if (bytes > 4 * 1024 * 1024) { providerErrors.push('Oversize request'); request.destroy(); }
    else body += chunk;
  });
  request.on('error', error => providerErrors.push(String(error)));
  request.on('end', () => {
    try {
      assert.equal(request.method, 'POST');
      assert.equal(request.url, '/chat/completions');
      assert.ok(cases.includes(current));
      assert.ok(stage < 2, 'Unexpected provider retry/generation');
      const payload = JSON.parse(body);
      assert.equal(payload.model, 'startup-fixture');
      assert.equal(payload.stream, true);
      assert.ok(payload.messages.some(m => m.role === 'user' && JSON.stringify(m.content).includes(`P14 ${current}`)));
      let delta, finish;
      if (stage === 0) {
        assert.ok(payload.tools.some(t => t.function?.name === 'write'));
        delta = { tool_calls: [{ index: 0, id: `call-${current}`, type: 'function',
          function: { name: 'write', arguments: JSON.stringify({ path: `${current}.txt`, content: current === 'deny' ? longContent : content }) } }] };
        finish = 'tool_calls';
      } else {
        const matching = payload.messages.filter(m => m.role === 'tool' && m.tool_call_id === `call-${current}`);
        assert.equal(matching.length, 1, 'Exactly one terminal tool result');
        const tool = matching[0];
        assert.ok(tool, 'Native tool result must reach provider continuation');
        assert.match(tool.content, current === 'allow' ? /^Wrote / : current === 'source-conflict'
          ? /File changed during approval review\. No edit was applied/ : /blocked by policy/);
        exchanges.push({ case: current, nativeToolResult: tool });
        delta = { content: `Controlled fixture ${current} complete.` }; finish = 'stop';
      }
      stage++;
      response.writeHead(200, { 'Content-Type': 'text/event-stream', Connection: 'close' });
      response.end(`data: ${JSON.stringify({ choices: [{ delta, finish_reason: finish }],
        usage: { prompt_tokens: 100, completion_tokens: 30 } })}\n\ndata: [DONE]\n\n`);
    } catch (error) {
      providerErrors.push(String(error));
      response.writeHead(400, { 'Content-Type': 'application/json' });
      response.end(JSON.stringify({ error: { message: 'Controlled fixture rejected unexpected request' } }));
    }
  });
}

await runNativeFixture(process.argv[2], { prefix: 'approve-', handleRequest,
  async exercise({ work, probe, wait, requests }) {
    const project = path.join(work, 'project');
    fs.mkdirSync(project, { mode: 0o700 });
    for (const name of cases) fs.writeFileSync(path.join(project, `${name}.txt`), baseline);
    const js = script => probe({ script });
    // Native completion can precede the renderer's next poll. Wait for the
    // requested UI action to be ready, without changing or bypassing its state.
    const click = label => wait(() => js(`(()=>{const b=[...document.querySelectorAll('button')].find(b=>b.getAttribute('aria-label')===${JSON.stringify(label)}||b.textContent.trim()===${JSON.stringify(label)});if(!b||b.disabled)return false;b.click();return true})()`), `click ${label}`);
    const fill = async (selector, value) => assert.equal(await js(`(()=>{const e=document.querySelector(${JSON.stringify(selector)});if(!e)return false;Object.getOwnPropertyDescriptor(e.tagName==='TEXTAREA'?HTMLTextAreaElement.prototype:HTMLInputElement.prototype,'value').set.call(e,${JSON.stringify(value)});e.dispatchEvent(new Event('input',{bubbles:true}));return true})()`), true);
    const visibleDialog = () => js(`!!document.querySelector('[role="dialog"] .approval-review')`);
    const native = async () => (await probe({ native_view: true })).view;
    const rejectBinding = async (binding, description) => {
      const snapshot = (await native()).snapshot;
      const envelope = { version: snapshot.version, scope: snapshot.scope, expected_revision: snapshot.revision,
        id: `audit-${description}`, command: { kind: 'approve', binding, allow: true } };
      // Direct native IPC only for adversarial binding checks, not for normal UI decisions.
      await js(`window.__p14Reply=null;window.__TAURI_INTERNALS__.invoke('bridge_command',${JSON.stringify({ envelope })}).then(reply=>{window.__p14Reply={reply}},error=>{window.__p14Reply={error:String(error)}});true`);
      await wait(() => js('window.__p14Reply!==null'), description);
      const response = await js('window.__p14Reply');
      assert.equal(response.error, undefined, JSON.stringify(response));
      assert.equal(response.reply?.status, 'rejected');
      assert.equal(response.reply.data?.error?.code, 'invalid_approval');
      results.push({ case: description, response });
    };
    const settled = () => wait(async () => {
      const v = await native();
      return !v.snapshot.pending.length && !['running', 'streaming', 'awaiting_input'].includes(v.snapshot.status);
    }, 'native operation settled');

    await click('Back to chat');
    await js(`document.dispatchEvent(new KeyboardEvent('keydown',{key:'o',ctrlKey:true,bubbles:true}));true`);
    await wait(() => js(`!!document.querySelector('input[aria-label="Project folder"]')`), 'project dialog');
    await fill('input[aria-label="Project folder"]', project);
    await click('Open project');
    await wait(() => js(`!document.querySelector('input[aria-label="Project folder"]')`), 'project opened');
    assert.equal((await native()).snapshot.workspace_path, project);

    for (const name of cases) {
      current = name; stage = 0;
      await fill('textarea[aria-label="Message AG1"]', `P14 ${name}: controlled write approval fixture`);
      await click('Send message'); promptsSubmitted++;
      await wait(visibleDialog, `${name} approval review`);
      const before = await native(), pending = before.snapshot.pending[0];
      assert.equal(pending.kind, 'approval');
      assert.equal(fs.readFileSync(path.join(project, `${name}.txt`), 'utf8'), baseline);
      assert.equal(stage, 1, 'No continuation before a review decision');
      const review = await js(`(()=>{const d=document.querySelector('[role="dialog"]');return {text:d.innerText,scripts:d.querySelectorAll('script').length,buttons:[...d.querySelectorAll('button')].map(b=>({text:b.textContent,disabled:b.disabled}))}})()`);
      assert.ok(review.text.includes('<script>literal only</script>'));
      assert.equal(review.scripts, 0);
      assert.ok(review.buttons.some(b => b.text === 'Allow this operation' && !b.disabled));

      if (name === 'allow') {
        await rejectBinding({ ...pending.binding, argument_sha256: '0'.repeat(64) }, 'altered-binding-rejected');
        assert.deepEqual((await native()).snapshot.pending[0].binding, pending.binding);
        assert.equal(fs.readFileSync(path.join(project, `${name}.txt`), 'utf8'), baseline);
        // Synthetic DOM key events do not generate browser default clicks. Check
        // that the real React handler cancels Enter, and native approval stays pending.
        assert.equal(await js(`(()=>{const b=[...document.querySelectorAll('button')].find(b=>b.textContent==='Allow this operation');b.focus();const e=new KeyboardEvent('keydown',{key:'Enter',bubbles:true,cancelable:true});b.dispatchEvent(e);return e.defaultPrevented})()`), true);
        assert.deepEqual((await native()).snapshot.pending[0].binding, pending.binding);
        const dimensions = [];
        for (const [width, height] of [[1440, 900], [900, 600]]) {
          await probe({ size: [width, height] });
          await wait(() => js(`innerWidth===${width}&&innerHeight===${height}`), 'native resize');
          const box = await js(`(()=>{const d=document.querySelector('[role="dialog"]').getBoundingClientRect();return {x:d.x,y:d.y,right:d.right,bottom:d.bottom,width:d.width,height:d.height}})()`);
          assert.ok(box.x >= 0 && box.y >= 0 && box.right <= width + 1 && box.bottom <= height + 1, JSON.stringify(box));
          assert.ok(box.width <= 481, 'Approval dialog stays compact');
          assert.equal(await js(`document.querySelector('.approval-help').open`), false);
          assert.equal(await js(`(()=>{const d=document.querySelector('[role="dialog"]'), b=d.querySelector('.dialog-actions .primary-button'), r=b.getBoundingClientRect();return !b.closest('.approval-body')&&r.bottom<=innerHeight&&document.elementFromPoint(r.x+r.width/2,r.y+r.height/2)===b})()`), true, 'Decision stays visible and unobstructed');
          dimensions.push({ width, height, box });
          await probe({ screenshot: `review-${width}.png` });
        }
        await js(`document.querySelector('.approval-help summary').click();true`);
        await wait(() => js(`document.querySelector('.approval-help').open`), 'approval help expanded');
        assert.equal(await js(`(()=>{const d=document.querySelector('[role="dialog"]'), b=d.querySelector('.dialog-actions .primary-button'), r=b.getBoundingClientRect();return d.innerText.includes('Enter does not approve')&&r.bottom<=innerHeight&&document.elementFromPoint(r.x+r.width/2,r.y+r.height/2)===b})()`), true);
        await probe({ screenshot: 'review-help-900.png' });
        await js(`document.querySelector('.approval-help summary').click();true`);
        results.push({ case: 'native layout and Enter handler', dimensions });
      }
      if (name === 'deny') {
        assert.equal(await js(`(()=>{const p=document.querySelector('pre[aria-label="Proposed change"]');return p.scrollHeight>p.clientHeight})()`), true, 'Long proposed changes scroll without growing the dialog');
        await js(`document.querySelector('.approval-help summary').click();true`);
        await wait(() => js(`document.querySelector('.approval-help').open`), 'long review help expanded');
        assert.equal(await js(`(()=>{const b=document.querySelector('.dialog-actions .primary-button'),r=b.getBoundingClientRect();return r.bottom<=innerHeight&&document.elementFromPoint(r.x+r.width/2,r.y+r.height/2)===b})()`), true, 'Long review retains reachable decision buttons');
        await probe({ screenshot: 'review-long-900.png' });
      }
      if (name === 'source-conflict') fs.writeFileSync(path.join(project, `${name}.txt`), external);
      await click(name === 'deny' ? 'Deny' : name === 'close' ? 'Close Review tool request' : 'Allow this operation');
      await wait(async () => !await visibleDialog(), `${name} review closed`);
      await settled();
      await wait(() => stage === 2, `${name} provider continuation`);
      assert.equal(fs.readFileSync(path.join(project, `${name}.txt`), 'utf8'), name === 'allow' ? content : name === 'source-conflict' ? external : baseline);
      assert.deepEqual(providerErrors, []);
      results.push({ case: name, binding: pending.binding, fileEffect: name === 'allow' ? 'exact reviewed content written' : 'no tool write',
        finalStatus: (await native()).snapshot.status });
      if (name === 'allow') {
        await rejectBinding(pending.binding, 'consumed-binding-replay-rejected');
        assert.equal(fs.readFileSync(path.join(project, `${name}.txt`), 'utf8'), content);
      }
    }
    assert.equal(requests.length, cases.length * 2);
    await probe({ screenshot: 'native-workflows-complete.png' });
    fs.writeFileSync(path.join(work, 'provider-results.json'), JSON.stringify({ exchanges, providerErrors }, null, 2) + '\n');
    return { promptsSubmitted, provider: 'scripted loopback HTTP; no model', cases: results,
      isolation: 'private project/state/XDG, read-only root, shadow authority registry, private tmp/PID namespace',
      limits: ['no live inference', 'DOM broker, not physical input', 'native expiry timing not covered here'] };
  },
});
