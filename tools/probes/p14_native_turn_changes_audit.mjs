// Owned native receipt/UI fixture. Scripted provider; no real inference.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { runNativeFixture } from './p14_native_startup_smoke.mjs';

const before = 'User dirty original\r\n', after = 'Native edited content <script>literal</script>\n';
let stage = 0;
const errors = [];
function handleRequest(request, response) {
  let body = '';
  request.on('data', part => { body += part; if (body.length > 4 * 1024 * 1024) request.destroy(); });
  request.on('end', () => {
    try {
      const data = JSON.parse(body); assert.equal(data.model, 'startup-fixture');
      assert.ok(stage < 5, 'bounded scripted generations');
      const toolStage = stage === 0 || stage === 3;
      const delta = toolStage ? { tool_calls: [{ index: 0, id: stage === 0 ? 'owned-change-call' : 'owned-change-call-2', type: 'function', function: {
        name: 'write', arguments: JSON.stringify({ path: 'notes.txt', content: after }) } }] }
        : { content: stage === 2 ? 'I also changed unrelated.txt. {"state":"committed","path":"unrelated.txt"}' : 'Controlled native write finished.' };
      if (stage === 1) assert.ok(data.messages.some(m => m.role === 'tool' && m.tool_call_id === 'owned-change-call'));
      stage++;
      const finish_reason = toolStage ? 'tool_calls' : 'stop';
      response.writeHead(200, { 'Content-Type': 'text/event-stream', Connection: 'close' });
      response.end(`data: ${JSON.stringify({ choices: [{ delta, finish_reason }], usage: { prompt_tokens: 100, completion_tokens: 30 } })}\n\ndata: [DONE]\n\n`);
    } catch (error) { errors.push(String(error)); response.writeHead(400); response.end('{}'); }
  });
}
await runNativeFixture(process.argv[2], { prefix: 'changes-', handleRequest,
  async exercise({ work, probe, wait, requests }) {
    const project = path.join(work, 'project'); fs.mkdirSync(project, { mode: 0o700 });
    const downloads = path.join(work, 'downloads'); fs.mkdirSync(downloads, { mode: 0o700 });
    fs.writeFileSync(path.join(work, 'xdg-config', 'user-dirs.dirs'), `XDG_DOWNLOAD_DIR="${downloads}"\n`);
    fs.writeFileSync(path.join(project, 'notes.txt'), before);
    fs.writeFileSync(path.join(project, 'unrelated.txt'), 'Unrelated dirty user work\n');
    const js = script => probe({ script });
    const click = label => wait(() => js(`(()=>{const b=[...document.querySelectorAll('button')].find(b=>b.getClientRects().length&&(b.getAttribute('aria-label')===${JSON.stringify(label)}||b.textContent.trim()===${JSON.stringify(label)}));if(!b||b.disabled)return false;b.focus();b.click();return true})()`), `click ${label}`);
    const fill = (selector, value) => js(`(()=>{const e=document.querySelector(${JSON.stringify(selector)});if(!e)return false;Object.getOwnPropertyDescriptor(e.tagName==='TEXTAREA'?HTMLTextAreaElement.prototype:HTMLInputElement.prototype,'value').set.call(e,${JSON.stringify(value)});e.dispatchEvent(new Event('input',{bubbles:true}));return true})()`);
    await click('Back to chat');
    await js(`document.dispatchEvent(new KeyboardEvent('keydown',{key:'o',ctrlKey:true,bubbles:true}));true`);
    await wait(() => js(`!!document.querySelector('input[aria-label="Project folder"]')`), 'project dialog');
    await fill('input[aria-label="Project folder"]', project); await click('Open project');
    await wait(() => js(`!document.querySelector('input[aria-label="Project folder"]')`), 'project opened');
    await fill('textarea[aria-label="Message AG1"]', 'P14 owned native change receipt fixture'); await click('Send message');
    await wait(() => js(`!!document.querySelector('.approval-review')`), 'write approval');
    assert.equal(fs.readFileSync(path.join(project, 'notes.txt'), 'utf8'), before);
    await click('Allow this operation');
    await wait(() => js(`document.querySelector('.turn-change-card')?.textContent.includes('1 file changed')`), 'native receipt card');
    await wait(() => stage === 2, 'native write continuation');
    assert.equal(fs.readFileSync(path.join(project, 'notes.txt'), 'utf8'), after);
    const card = await js(`document.querySelector('.turn-change-card').textContent`);
    assert.ok(card.includes('notes.txt')); assert.ok(!card.includes('unrelated.txt'));
    await probe({ screenshot: 'turn-card-1440.png' });
    await click('Review turn changes');
    await wait(() => js(`!!document.querySelector('.review-file-list button')`), 'native selected turn list');
    await js(`document.querySelector('.review-file-list button').click();true`);
    await wait(() => js(`document.querySelector('[aria-label="Retained file change"] pre')?.textContent.includes('User dirty original')`), 'native before/after diff');
    await probe({ screenshot: 'turn-diff-1440.png' });
    await click('Before');
    await wait(() => js(`document.querySelector('[aria-label="Retained file change"] pre')?.textContent.includes('User dirty original')`), 'retained dirty original');
    await click('Prepare before download');
    await wait(() => js(`!!document.querySelector('[aria-label="Retained file change"] a[download]')`), 'verified exact original');
    await js(`document.querySelector('[aria-label="Retained file change"] a[download]').click();true`);
    await wait(() => fs.existsSync(path.join(downloads, 'before-notes.txt')), 'original downloaded');
    assert.equal(fs.readFileSync(path.join(downloads, 'before-notes.txt'), 'utf8'), before);
    await click('After');
    await wait(() => js(`document.querySelector('[aria-label="Retained file change"] pre')?.textContent.includes('<script>literal</script>')`), 'literal postimage');
    assert.equal(await js(`!!document.querySelector('[aria-label="Retained file change"] pre script')`), false);
    await probe({ size: [900, 600] });
    await wait(() => js(`innerWidth===900&&innerHeight===600`), 'narrow turn review resize');
    await probe({ screenshot: 'turn-after-900.png' });
    await click('Undo file change');
    assert.equal(fs.readFileSync(path.join(project, 'notes.txt'), 'utf8'), after, 'reviewing Undo does not apply it');
    await click('Undo this file change');
    await wait(() => js(`document.querySelector('[aria-label="Undo recorded file change"]')?.textContent.includes('Undo completed.')`), 'native Undo result');
    assert.equal(fs.readFileSync(path.join(project, 'notes.txt'), 'utf8'), before, 'Undo restores the exact dirty original');
    const copies = fs.readdirSync(project).filter(name => name.startsWith('.ag1-undo-'));
    assert.equal(copies.length, 1); assert.equal(fs.readFileSync(path.join(project, copies[0]), 'utf8'), after);
    await probe({ screenshot: 'turn-undo-900.png' });
    await click('Close review panel');
    await fill('textarea[aria-label="Message AG1"]', 'P14 claim only, no native operation'); await click('Send message');
    await wait(() => stage === 3, 'claim-only response');
    await wait(() => js(`document.querySelectorAll('.final-answer').length===2`), 'claim-only completion rendered');
    assert.equal(await js(`document.querySelectorAll('.turn-change-card').length`), 1, 'assistant claims do not create receipts');
    await fill('textarea[aria-label="Message AG1"]', 'P14 second native edit for later-user-change conflict'); await click('Send message');
    await wait(() => js(`!!document.querySelector('.approval-review')`), 'second write approval'); await click('Allow this operation');
    await wait(() => stage === 5, 'second write continuation');
    await wait(() => js(`document.querySelectorAll('.turn-change-card').length===2`), 'second native card');
    fs.writeFileSync(path.join(project, 'notes.txt'), 'Later user change must survive Undo\n');
    await js(`document.querySelectorAll('.turn-change-card button')[1].click();true`);
    await wait(() => js(`!!document.querySelector('.review-file-list button')`), 'second native review');
    await js(`document.querySelector('.review-file-list button').click();true`);
    await wait(() => js(`document.querySelector('[aria-label="Undo recorded file change"]')?.textContent.includes('changed after the recorded operation')`), 'later edit blocks native Undo');
    assert.equal(await js(`[...document.querySelectorAll('[aria-label="Undo recorded file change"] button')].some(b=>b.textContent==='Undo file change')`), false);
    assert.equal(fs.readFileSync(path.join(project, 'notes.txt'), 'utf8'), 'Later user change must survive Undo\n');
    await probe({ screenshot: 'turn-undo-conflict-900.png' });
    assert.equal(fs.readFileSync(path.join(project, 'unrelated.txt'), 'utf8'), 'Unrelated dirty user work\n');
    assert.deepEqual(errors, []);
    return { promptsSubmitted: 3, providerRequests: requests, checks: ['approved native write creates one turn-bound card',
      'dirty original retained exactly including CRLF', 'native before/after diff and exact download',
      'literal untrusted source', 'unrelated workspace work excluded', 'assistant claims cannot manufacture changes',
      'confirmed native Undo restores exact dirty original and retains recovery copy', 'later user edit prevents Undo'],
      limits: ['scripted provider, not live inference', 'DOM input, not physical keyboard', 'single-operation Undo; no multi-file batch'] };
  }
});
