// Owned native WebKit fixture: manual setup, explicit connection consent,
// discovery-only inspection and scope revocation. No inference or live accounts.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { runNativeFixture } from './p14_native_startup_smoke.mjs';
const source = fileURLToPath(new URL('../../product/crates/ag1-platform/tests/fixtures/mcp_content.py', import.meta.url));
await runNativeFixture(process.argv[2], {
  prefix: 'mcp-',
  async exercise({ work, probe, wait, requests }) {
    const js = script => probe({ script });
    const click = label => wait(() => js(`(()=>{const e=[...document.querySelectorAll('button')].find(e=>e.getAttribute('aria-label')===${JSON.stringify(label)}||e.textContent.trim()===${JSON.stringify(label)});if(!e||e.disabled)return false;e.click();return true})()`), `click ${label}`);
    const text = value => wait(() => js(`document.body.innerText.includes(${JSON.stringify(value)})`), value);
    const fill = async (label, value) => assert.equal(await js(`(()=>{const l=[...document.querySelectorAll('.mcp-form label')].find(e=>e.textContent.startsWith(${JSON.stringify(label)}));const e=l?.querySelector('input,textarea,select');if(!e)return false;const p=e.tagName==='TEXTAREA'?HTMLTextAreaElement.prototype:e.tagName==='SELECT'?HTMLSelectElement.prototype:HTMLInputElement.prototype;Object.getOwnPropertyDescriptor(p,'value').set.call(e,${JSON.stringify(value)});e.dispatchEvent(new Event(e.tagName==='SELECT'?'change':'input',{bubbles:true}));return true})()`), true);
    const peerRoot = path.join(work, 'peer'); fs.mkdirSync(peerRoot);
    const peerScript = path.join(peerRoot, 'peer.py');
    // Extend only this owned copy so the native audit inspects all four catalogs.
    const fixture = fs.readFileSync(source,'utf8')
      .replace("caps = {'resources':{}, 'prompts':{}}", "caps = {'tools':{}, 'resources':{}, 'prompts':{}}")
      .replace("    elif method in ('resources/list','resources/templates/list','prompts/list'):", "    elif method == 'tools/list':\n        result = {'tools':[{'name':'owned_tool','description':'Literal <script>peer metadata</script>','inputSchema':{'type':'object','properties':{'value':{'type':'integer'}}}}]}\n    elif method in ('resources/list','resources/templates/list','prompts/list'):");
    fs.writeFileSync(peerScript,fixture);
    fs.writeFileSync(path.join(peerRoot, 'mode'), 'content_ok');
    const log = fs.openSync(path.join(peerRoot, 'stderr.log'), 'wx');
    const peer = spawn('/usr/bin/python3', ['-I', peerScript, 'http', '2026-07-28', peerRoot], { stdio: ['ignore', log, log] });
    fs.closeSync(log);
    const events = () => fs.existsSync(path.join(peerRoot, 'events.jsonl')) ? fs.readFileSync(path.join(peerRoot, 'events.jsonl'), 'utf8').trim().split('\n').filter(Boolean).map(JSON.parse) : [];
    try {
      await wait(() => fs.existsSync(path.join(peerRoot, 'port')), 'owned HTTP peer');
      const url = `http://127.0.0.1:${fs.readFileSync(path.join(peerRoot, 'port'), 'utf8')}/mcp`;
      await click('Back to chat'); await click('MCP');
      await text('No servers installed.');
      assert.equal((await probe({ native_view: true })).view.snapshot.fixture_provider, true);
      await probe({ screenshot: 'mcp-empty.png' });
      for (const transport of ['http', 'local']) {
        const before = events().length;
        await click('Add server'); await fill('Display name', `Owned ${transport}`);
        await fill('Connection type', transport);
        if (transport === 'http') {
          await fill('MCP URL', url);
          await js(`document.querySelector('.mcp-form input[type=checkbox]').click();true`);
        } else {
          await fill('Executable', '/usr/bin/python3');
          await fill('Arguments', JSON.stringify(['-I', peerScript, 'stdio', '2026-07-28', peerRoot]));
          await fill('Working directory', peerRoot);
        }
        await probe({ screenshot: `mcp-${transport}-form.png` });
        await click('Save inactive'); await text('Disabled for this scope');
        assert.equal(events().length,before,'Saving must not contact or launch a server');
        await click('Test connection'); await text('Review connection');
        assert.equal(events().length,before,'Review must not contact or launch a server');
        assert.equal(await js(`document.querySelector('.mcp-review .primary-button').disabled`),true);
        await probe({ screenshot: `mcp-${transport}-review.png` });
        await click('Cancel review');
        assert.equal(events().length,before,'Cancelling review is inert');
        await click('Test connection'); await text('Review connection');
        await js(`document.querySelector('.mcp-review input[type=checkbox]').click();true`);
        await click('Connect reviewed target'); await text('ready · Inspection only');
        for (const category of ['Tools','Resources','Templates','Prompts']) {
          await click(category); await wait(()=>js(`document.querySelector('.mcp-tabs button[aria-pressed=true]')?.textContent===${JSON.stringify(category)}&&!document.querySelector('.mcp-manager [role=status]')`),'catalog '+category); await text('entries · Protocol');
          const count = await js(`document.querySelectorAll('.mcp-capabilities button').length`);
          if (count) {
            await wait(()=>js(`(()=>{const e=document.querySelector('.mcp-capabilities button');if(!e||e.disabled)return false;e.click();return true})()`),'open capability details');
            await text('Capability details');
          }
          await js(`document.querySelector('[aria-label="MCP capabilities"]').scrollIntoView({block:'start'});true`);
          await probe({ screenshot: `mcp-${transport}-${category.toLowerCase()}.png` });
        }
        assert.ok(events().length>before);
        assert.ok(events().every(e=>['initialize','notifications/initialized','server/discover','tools/list','resources/list','resources/templates/list','prompts/list'].includes(e.method)), 'Inspection must never invoke a capability');
        await probe({ size: [900,600] });
        await wait(()=>js('innerWidth===900&&innerHeight===600'),'narrow MCP window');
        await js(`document.querySelector('.mcp-manager').scrollTop=0;true`);
        await probe({ screenshot: `mcp-${transport}-900.png` });
        await js(`document.querySelector('[aria-label="MCP capabilities"]').scrollIntoView({block:'start'});true`);
        await probe({ screenshot: `mcp-${transport}-detail-900.png` });
        assert.equal(await js('document.documentElement.scrollWidth<=innerWidth'),true);
        await click('Disconnect'); await text('Connect or test the server');
        await click('Enable here'); await text('Enabled for this scope');
        await click('Connect'); await text('Review connection');
        await js(`document.querySelector('.mcp-review input[type=checkbox]').click();true`);
        await click('Connect reviewed target'); await text('ready · This chat');
        await click('Disable here'); await text('Disabled for this scope');
        await wait(()=>js(`!document.querySelector('.mcp-connection')?.textContent.includes('ready')`),'scope revocation');
        await click('Remove server'); await click('Confirm removal'); await text('No servers installed.');
        await probe({ size: [1440,900] });
      }
      assert.equal(requests.length,0,'Manager workflow must not start inference');
      assert.equal(fs.existsSync(path.join(peerRoot,'effects')),false);
      fs.writeFileSync(path.join(work,'mcp-peer-events.json'),JSON.stringify(events(),null,2));
      return { promptsSubmitted:0, providerRequests:[], checks:['native empty state','manual HTTP and stdio save inactive','inert cancelled review','explicit consent','inspection-only connection','tool/resource/template/prompt detail','900x600 and 1440x900','enable/connect/disable/remove','no capability calls or inference'], limits:['DOM-driven diagnostic; screenshots require separate inspection','private credentials and registry discovery are later checkpoints'] };
    } finally {
      if (peer.exitCode===null && peer.signalCode===null) {
        const exited = new Promise(resolve=>peer.once('exit',resolve)); peer.kill('SIGTERM');
        const timer=setTimeout(()=>peer.kill('SIGKILL'),3000);
        await exited; clearTimeout(timer);
      }
    }
  },
});
