// Owned manual credential workflow. Private values never enter a WebKit script.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { spawn, execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { runNativeFixture } from './p14_native_startup_smoke.mjs';
const root=fileURLToPath(new URL('../../',import.meta.url));
const helper=path.join(root,'tools/probes/mcp_private_input.py');
await runNativeFixture(process.argv[2],{
  prefix:'cred-',
  async exercise({work,probe,wait,requests}) {
    const js=script=>probe({script});
    const input=(action,...args)=>JSON.parse(execFileSync('/usr/bin/python3',[helper,work,action,...args],{encoding:'utf8',timeout:25000}));
    const click=label=>wait(()=>js(`(()=>{const e=[...document.querySelectorAll('button')].find(e=>e.getAttribute('aria-label')===${JSON.stringify(label)}||e.textContent.trim()===${JSON.stringify(label)});if(!e||e.disabled)return false;e.click();return true})()`),`click ${label}`);
    const text=value=>wait(()=>js(`document.body.innerText.includes(${JSON.stringify(value)})`),value);
    const fill=async(label,value)=>assert.equal(await js(`(()=>{const l=[...document.querySelectorAll('.mcp-form label')].find(e=>e.textContent.startsWith(${JSON.stringify(label)}));const e=l?.querySelector('input,textarea,select');if(!e)return false;const p=e.tagName==='TEXTAREA'?HTMLTextAreaElement.prototype:e.tagName==='SELECT'?HTMLSelectElement.prototype:HTMLInputElement.prototype;Object.getOwnPropertyDescriptor(p,'value').set.call(e,${JSON.stringify(value)});e.dispatchEvent(new Event(e.tagName==='SELECT'?'change':'input',{bubbles:true}));return true})()`),true);
    const peerRoot=path.join(work,'peer');fs.mkdirSync(peerRoot);
    const source=fs.readFileSync(path.join(root,'product/crates/ag1-platform/tests/fixtures/mcp_content.py'),'utf8');
    const fixture=source.replace("caps = {'resources':{}, 'prompts':{}}","caps = {'tools':{}, 'resources':{}, 'prompts':{}}")
      .replace("    elif method in ('resources/list','resources/templates/list','prompts/list'):","    elif method == 'tools/list':\n        result = {'tools':[{'name':'owned_tool','inputSchema':{'type':'object'}}]}\n    elif method in ('resources/list','resources/templates/list','prompts/list'):")
      .replace('list_count = {}',"list_count = {}\nif transport == 'stdio':\n    assert os.environ.get('TOKEN') == (root / 'expected-token').read_text()\n    (root / 'local-private-observed').write_text('matched')")
      .replace('        def do_POST(self):',"        def do_POST(self):\n            if self.headers.get('Authorization') != 'Bearer ' + (root / 'expected-token').read_text():\n                self.send_response(401);self.send_header('Content-Length','0');self.end_headers();return\n            with (root / 'authenticated-requests').open('a') as proof:\n                proof.write('matched\\n')");
    const peerScript=path.join(peerRoot,'peer.py');fs.writeFileSync(peerScript,fixture);fs.writeFileSync(path.join(peerRoot,'mode'),'content_ok');
    // Owned synthetic fixture data stays in private files and XTest input.
    const value='owned-native-mcp-token-2026';
    fs.writeFileSync(path.join(peerRoot,'expected-token'),value,{mode:0o600});fs.writeFileSync(path.join(work,'private-input'),value,{mode:0o600});
    const log=fs.openSync(path.join(peerRoot,'stderr.log'),'wx',0o600);
    const peer=spawn('/usr/bin/python3',['-I',peerScript,'http','2026-07-28',peerRoot],{stdio:['ignore',log,log]});fs.closeSync(log);
    const events=()=>fs.existsSync(path.join(peerRoot,'events.jsonl'))?fs.readFileSync(path.join(peerRoot,'events.jsonl'),'utf8').trim().split('\n').filter(Boolean).map(JSON.parse):[];
    try {
      await wait(()=>fs.existsSync(path.join(peerRoot,'port')),'private HTTP peer');
      await click('Back to chat');await click('MCP');await text('No servers installed.');
      for(const transport of ['http','local']) {
        await click('Add server');await fill('Display name',`Private ${transport}`);
        if(transport==='http') {await fill('MCP URL',`http://127.0.0.1:${fs.readFileSync(path.join(peerRoot,'port'),'utf8')}/mcp`);await js(`document.querySelector('.mcp-form input[type=checkbox]').click();true`);}
        else {await fill('Connection type','local');await fill('Executable','/usr/bin/python3');await fill('Arguments',JSON.stringify(['-I',peerScript,'stdio','2026-07-28',peerRoot]));await fill('Working directory',peerRoot);await fill('Private environment names','TOKEN');}
        await click('Save inactive');await text('No private credentials configured');
        const before=events().length;
        await click('Enter private values');await wait(()=>input('status').dialog,'GTK private dialog');
        assert.equal(await js(`document.querySelector('input[type=password]')===null`),true);
        input('screenshot',`private-${transport}-dialog.png`);
        input('cancel');await wait(()=>!input('status').dialog,'GTK cancel');
        await text('No private credentials configured');assert.equal(events().length,before);
        await click('Enter private values');await wait(()=>input('status').dialog,'GTK private reentry');
        input('save');await wait(()=>!input('status').dialog,'GTK save');await text('Private credentials saved');assert.equal(events().length,before);
        assert.equal((await js("document.body.innerText")).includes(value),false);
        await click('Enable here');await click('Connect');await text('Review connection');
        await js(`document.querySelector('.mcp-review input[type=checkbox]').click();true`);await click('Connect reviewed target');await text('ready · This chat');
        await click('Tools');await text('entries · Protocol');await js(`document.querySelector('[aria-label="MCP authentication"]').scrollIntoView({block:'start'});true`);
        await probe({screenshot:`private-${transport}-connected.png`});
        assert.ok(events().length>before);
        await click('Log out locally');await text('Logged out locally');await wait(()=>js(`!document.querySelector('.mcp-connection')?.textContent.includes('ready')`),'logout retires connection');
        await click('Disconnect');
        const loggedOut=events().length;await click('Connect');await text('Use Replace private values, then review the connection again.');assert.equal(events().length,loggedOut);
        await probe({size:[900,600]});await wait(()=>js(`(()=>{if(innerWidth!==900)return false;const e=document.querySelector('[aria-label="MCP authentication"]');e.scrollIntoView({block:'start'});return e.getBoundingClientRect().top<250})()`),'authentication scrolled into view');await probe({screenshot:`private-${transport}-logged-out-900.png`});assert.equal(await js('document.documentElement.scrollWidth<=innerWidth'),true);
        await click('Remove server');await click('Confirm removal');await text('No servers installed.');await probe({size:[1440,900]});
      }
      assert.ok(fs.existsSync(path.join(peerRoot,'local-private-observed')));assert.ok(fs.existsSync(path.join(peerRoot,'authenticated-requests')));
      assert.ok(events().every(e=>['initialize','notifications/initialized','server/discover','tools/list','resources/list','resources/templates/list','prompts/list'].includes(e.method)));
      assert.equal(requests.length,0);fs.writeFileSync(path.join(work,'private-peer-events.json'),JSON.stringify(events(),null,2));
      return {providerRequests:[],promptsSubmitted:0,checks:['actual GTK private entry, cancel and save','no renderer password field','inert credential save','authenticated HTTP and local environment injection','explicit activation review','logout retires connections and blocks unauthenticated fallback','900x600 and 1440x900','no server capability calls or inference'],limits:['Synthetic XTest input on an owned display; screenshots require inspection','OAuth and trust are still pending']};
    } finally {
      if(peer.exitCode===null&&peer.signalCode===null){peer.kill('SIGTERM');await new Promise(resolve=>peer.once('exit',resolve));}
    }
  }
});
