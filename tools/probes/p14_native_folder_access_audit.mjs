// Hidden native WebKit UI, private folders/state, one fixed provider response.
// No real model, installed-app interaction, or physical-input acceptance claim.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { runNativeFixture } from './p14_native_startup_smoke.mjs';
let responses = 0;
await runNativeFixture(process.argv[2], {
  prefix: 'folders-',
  handleRequest(request, response) {
    let body = '';
    request.on('data', data => { body += data; if (body.length > 4 * 1024 * 1024) request.destroy(); });
    request.on('end', () => {
      assert.equal(++responses, 1);
      assert.equal(request.url, '/chat/completions');
      assert.ok(JSON.parse(body).messages.some(m => m.role === 'user' && JSON.stringify(m.content).includes('P14 folder history fixture')));
      response.writeHead(200, { 'Content-Type': 'text/event-stream', Connection: 'close' });
      response.end('data: {"choices":[{"delta":{"content":"Folder history stays in this conversation."},"finish_reason":null}]}\n\ndata: {"choices":[{"delta":{},"finish_reason":"stop"}]}\n\ndata: [DONE]\n\n');
    });
  },
  async exercise({ work, probe, wait, requests }) {
    const original = path.join(work, 'original'), extra = path.join(work, 'reference folder');
    fs.mkdirSync(original); fs.mkdirSync(extra);
    fs.writeFileSync(path.join(original, 'original.txt'), 'Original folder sentinel');
    fs.writeFileSync(path.join(extra, 'reference.txt'), 'Reference folder sentinel');
    const js = script => probe({ script });
    const native = async () => (await probe({ native_view: true })).view;
    const click = label => wait(() => js(`(()=>{const e=[...document.querySelectorAll('button')].find(e=>e.getAttribute('aria-label')===${JSON.stringify(label)}||e.textContent.trim()===${JSON.stringify(label)});if(!e||e.disabled)return false;e.click();return true})()`), `click ${label}`);
    const fill = async (selector, value) => assert.equal(await js(`(()=>{const e=document.querySelector(${JSON.stringify(selector)});if(!e)return false;const p=e.tagName==='TEXTAREA'?HTMLTextAreaElement.prototype:e.tagName==='SELECT'?HTMLSelectElement.prototype:HTMLInputElement.prototype;Object.getOwnPropertyDescriptor(p,'value').set.call(e,${JSON.stringify(value)});e.dispatchEvent(new Event(e.tagName==='SELECT'?'change':'input',{bubbles:true}));return true})()`), true);
    await click('Back to chat');
    await js(`document.dispatchEvent(new KeyboardEvent('keydown',{key:'o',ctrlKey:true,bubbles:true}));true`);
    await wait(() => js(`!!document.querySelector('input[aria-label="Project folder"]')`), 'project dialog');
    await fill('input[aria-label="Project folder"]', original); await click('Open project');
    await wait(async () => (await native()).snapshot.workspace_path === original, 'original project');
    await fill('textarea[aria-label="Message AG1"]', 'P14 folder history fixture'); await click('Send message');
    await wait(async () => (await native()).snapshot.status === 'idle' && responses === 1, 'fixed answer');
    const before = (await native()).snapshot;
    await fill('textarea[aria-label="Message AG1"]', 'Keep this unsent draft');
    await click('Chat folders');
    await wait(() => js(`!!document.querySelector('#chat-folder-path')`), 'folder controls');
    await fill('#chat-folder-path', extra); await click('Add folder');
    await wait(async () => (await native()).details.context.folder_access.folders.length === 2, 'read-only grant');
    assert.equal((await native()).details.context.folder_access.folders[1].mode, 'read_only');
    await probe({ screenshot: 'folders-readonly.png' });
    await fill(`select[aria-label=${JSON.stringify(`Access for ${extra}`)}]`, 'read_write');
    await wait(async () => (await native()).details.context.folder_access.folders[1].mode === 'read_write', 'write grant');
    await click('Make working folder');
    await wait(async () => (await native()).details.context.folder_access.primary === extra, 'new primary');
    await probe({ size: [900, 600] });
    await wait(() => js('innerWidth===900&&innerHeight===600'), 'narrow window');
    await probe({ screenshot: 'folders-primary-900.png' });
    await click('Close Chat folders');
    const after = (await native()).snapshot;
    assert.equal(after.scope.workspace, before.scope.workspace); assert.equal(after.scope.session, before.scope.session);
    assert.notEqual(after.scope.epoch, before.scope.epoch);
    assert.deepEqual(after.messages, before.messages);
    assert.equal(await js(`document.querySelector('textarea[aria-label="Message AG1"]').value`), 'Keep this unsent draft');
    await click('Files'); await click('reference.txt');
    await wait(() => js(`document.querySelector('.file-name')?.textContent==='reference.txt'`), 'primary files');
    await click('Chat folders');
    await click('Make working folder');
    await wait(async () => (await native()).details.context.folder_access.primary === original, 'original primary restored');
    await click(`Remove ${extra}`);
    await wait(async () => (await native()).details.context.folder_access.folders.length === 1, 'grant revoked');
    assert.equal(fs.readFileSync(path.join(original, 'original.txt'), 'utf8'), 'Original folder sentinel');
    assert.equal(fs.readFileSync(path.join(extra, 'reference.txt'), 'utf8'), 'Reference folder sentinel');
    assert.equal(requests.length, 1);
    return { promptsSubmitted: 1, provider: 'one fixed loopback response; no model', conversationPreserved: true, draftPreserved: true, checks: ['default read-only grant', 'read/write change', 'primary folder change', 'editor follows primary', 'revocation', '900x600 layout'], limits: ['DOM-driven diagnostic, not physical input', 'shell/file effects tested separately in backend contracts'] };
  },
});
