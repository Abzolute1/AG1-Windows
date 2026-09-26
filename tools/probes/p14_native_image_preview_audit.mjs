// Native WebKit + real file tools, fixed loopback provider. No real model.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { runNativeFixture } from './p14_native_startup_smoke.mjs';

const svg = '<svg xmlns="http://www.w3.org/2000/svg" width="320" height="240" viewBox="0 0 320 240"><rect width="320" height="240" fill="#e0f4ff"/><circle cx="160" cy="120" r="91" fill="white" stroke="#202a35" stroke-width="5"/><path d="M160 77 L201 107 L185 154 L135 154 L119 107 Z" fill="#202a35"/><path d="M160 29 L160 77 M245 93 L201 107 M212 194 L185 154 M108 194 L135 154 M75 93 L119 107" stroke="#202a35" stroke-width="5"/></svg>';
let stage = 0;
const errors = [];
await runNativeFixture(process.argv[2], {
  prefix: 'images-',
  handleRequest(request, response) {
    let body = '';
    request.on('data', chunk => { body += chunk; if (body.length > 8 * 1024 * 1024) request.destroy(); });
    request.on('end', () => {
      try {
        const payload = JSON.parse(body);
        assert.equal(request.url, '/chat/completions');
        assert.ok(stage < 3);
        assert.ok(payload.messages.some(m => m.role === 'user' && JSON.stringify(m.content).includes('P14 local image preview fixture')));
        let delta, finish;
        if (stage < 2) {
          const name = stage === 0 ? 'write' : 'read';
          assert.ok(payload.tools.some(t => t.function?.name === name));
          const args = stage === 0 ? { path: 'soccer-ball.svg', content: svg } : { path: 'soccer-ball.svg' };
          delta = { tool_calls: [{ index: 0, id: `image-${stage}`, type: 'function', function: { name, arguments: JSON.stringify(args) } }] };
          finish = 'tool_calls';
        } else {
          delta = { content: 'Created the SVG and attached its local preview.' }; finish = 'stop';
        }
        stage++;
        response.writeHead(200, { 'Content-Type': 'text/event-stream', Connection: 'close' });
        response.end(`data: ${JSON.stringify({ choices: [{ delta, finish_reason: finish }] })}\n\ndata: [DONE]\n\n`);
      } catch (error) {
        errors.push(String(error)); response.writeHead(400); response.end('Unexpected fixture request');
      }
    });
  },
  async exercise({ work, probe, wait, requests }) {
    const project = path.join(work, 'project'); fs.mkdirSync(project);
    const downloads = path.join(work, 'downloads'); fs.mkdirSync(downloads);
    fs.writeFileSync(path.join(work, 'xdg-config', 'user-dirs.dirs'), `XDG_DOWNLOAD_DIR="${downloads}"\n`);
    const js = script => probe({ script });
    const native = async () => (await probe({ native_view: true })).view;
    const click = label => wait(() => js(`(()=>{const b=[...document.querySelectorAll('button')].find(b=>b.getAttribute('aria-label')===${JSON.stringify(label)}||b.textContent.trim()===${JSON.stringify(label)});if(!b||b.disabled)return false;b.click();return true})()`), `click ${label}`);
    const fill = (selector, value) => js(`(()=>{const e=document.querySelector(${JSON.stringify(selector)});if(!e)return false;Object.getOwnPropertyDescriptor(e.tagName==='TEXTAREA'?HTMLTextAreaElement.prototype:HTMLInputElement.prototype,'value').set.call(e,${JSON.stringify(value)});e.dispatchEvent(new Event('input',{bubbles:true}));return true})()`);
    await click('Back to chat');
    await js(`document.dispatchEvent(new KeyboardEvent('keydown',{key:'o',ctrlKey:true,bubbles:true}));true`);
    await wait(() => js(`!!document.querySelector('input[aria-label="Project folder"]')`), 'open project dialog');
    await fill('input[aria-label="Project folder"]', project); await click('Open project');
    await wait(() => js(`!document.querySelector('input[aria-label="Project folder"]')`), 'project dialog closed');
    await wait(async () => (await native()).snapshot.workspace_path === project, 'project selected');
    await fill('textarea[aria-label="Message AG1"]', 'P14 local image preview fixture: create and show a soccer-ball SVG');
    await click('Send message'); await click('Allow this operation');
    await wait(async () => stage === 3 && (await native()).snapshot.status === 'idle', 'write/read completion');
    await wait(() => js(`(()=>{const i=document.querySelector('.image-results img');return !!i&&i.complete&&i.naturalWidth===320})()`), 'inline raster preview');
    assert.deepEqual(errors, []);
    assert.equal(fs.readFileSync(path.join(project, 'soccer-ball.svg'), 'utf8'), svg);
    const results = (await native()).details.messages.filter(m => m.role === 'toolResult');
    assert.equal(results.length, 2); assert.ok(results.every(m => !m.tool.failed));
    assert.ok(results.some(m => m.blocks.some(b => b.kind === 'attachment' && b.media_type === 'image/png')));
    assert.equal(await js(`document.querySelectorAll('.tool-output-view').length`), 0);
    assert.equal(await js(`document.querySelectorAll('.image-results img').length`), 1);
    const geometry = await js(`(()=>{const i=document.querySelector('.image-results img'),r=i.getBoundingClientRect();return {width:r.width,height:r.height,naturalWidth:i.naturalWidth,naturalHeight:i.naturalHeight}})()`);
    assert.ok(geometry.width >= 300); assert.equal(geometry.naturalHeight, 240);
    await probe({ screenshot: 'inline-image.png' });
    assert.equal(await js(`document.querySelector('.image-source-download').download`), 'soccer-ball.svg');
    await js(`document.querySelector('.image-source-download').click();true`);
    await wait(() => fs.existsSync(path.join(downloads, 'soccer-ball.svg')), 'source download');
    assert.equal(fs.readFileSync(path.join(downloads, 'soccer-ball.svg'), 'utf8'), svg);
    await click('Preview soccer-ball.svg preview');
    await wait(() => js(`!!document.querySelector('dialog.image-preview[open]')`), 'zoom dialog');
    await click('View at full resolution'); await probe({ screenshot: 'image-zoom.png' }); await click('Close image preview');
    await probe({ size: [900, 600] }); await wait(() => js('innerWidth===900&&innerHeight===600'), 'small viewport');
    await js(`document.querySelector('.image-results img').scrollIntoView({block:'start'});true`);
    await wait(() => js(`(()=>{const r=document.querySelector('.image-results img').getBoundingClientRect(),s=document.querySelector('.conversation-scroll').getBoundingClientRect();return r.top>=s.top-1&&r.bottom<=s.bottom+1&&r.right<=s.right})()`), 'entire image reachable in small viewport');
    await probe({ screenshot: 'inline-image-900.png' });
    assert.equal(await js(`document.querySelectorAll('iframe').length`), 0);
    assert.equal(requests.length, 3);
    return { promptsSubmitted: 1, provider: 'three fixed loopback responses; no model', geometry,
      checks: ['approved SVG creation with exact bytes', 'native read renders PNG automatically', 'image visible with collapsed activity', 'original SVG download payload', 'zoom/full-resolution/close', '900x600 layout'],
      limits: ['DOM-driven diagnostic, not physical input', 'restricted vector SVG subset; no scripts, external resources, text, use, filters or embedded images'] };
  },
});
