import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import {runNativeFixture} from './native-mcp-harness.mjs';
const candidate='/home/alex/Projects/AG1/.local/p14-live-fixes/native-candidate-f9';
const bin=path.join(candidate,'installation/releases/current/bin');
fs.mkdirSync(bin,{recursive:true});
for(const name of ['ag1-desktop','ag1-process-host']) fs.copyFileSync(path.join(candidate,name),path.join(bin,name));
await runNativeFixture(candidate,{
 prefix:'mcp-',
 async exercise({work,probe,wait,requests}) {
  const click=async name=>assert.equal(await probe({script:`(()=>{const b=[...document.querySelectorAll('button')].find(b=>b.textContent.trim()===${JSON.stringify(name)}||b.getAttribute('aria-label')===${JSON.stringify(name)});if(!b)return false;b.click();return true})()`}),true);
  await click('Back to chat');await click('MCP');
  await wait(()=>probe({script:'document.body.innerText.includes("No servers installed.")'}),'MCP opens immediately after upgrade');
  const view=await probe({script:'({text:document.body.innerText,alerts:[...document.querySelectorAll("[role=alert]")].map(e=>e.textContent)})'});
  assert.ok(!view.text.includes('NotFound'));assert.deepEqual(view.alerts,[]);assert.deepEqual(requests,[]);
  await probe({screenshot:'mcp-upgrade.png'});
  return {checks:['removed release helper in saved configuration','actual MCP sidebar opens real native manager','no NotFound or alerts','no provider requests'],providerRequests:requests,promptsSubmitted:0};
 }
});
