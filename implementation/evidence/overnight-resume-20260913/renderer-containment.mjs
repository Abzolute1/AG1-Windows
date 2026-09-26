import fs from 'node:fs/promises';
import {chromium} from '/home/alex/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/node_modules/playwright-core/index.mjs';
const browser=await chromium.launch({headless:true,executablePath:'/home/alex/.cache/ms-playwright/chromium-1234/chrome-linux64/chrome',args:['--disable-gpu']});
const page=await browser.newPage({viewport:{width:1600,height:1000}}),errors=[];
page.on('pageerror',e=>errors.push(e.message));
await page.route('**/*',r=>r.request().url().startsWith('http://127.0.0.1:1433/')?r.continue():r.abort());
await page.goto('http://127.0.0.1:1433/');await page.waitForFunction(()=>!!window.benchmark);
const rows=[];
for(const contain of [false,true]){
 if(contain)await page.addStyleTag({content:'.conversation-content > .chat-turn{content-visibility:auto;contain-intrinsic-block-size:auto 240px}'});
 for(const count of [50,500,2000]){
  const samples=[];
  for(let trial=0;trial<3;trial++){
   const render_ms=await page.evaluate(n=>window.benchmark.reset(n),count);
   await page.evaluate(()=>new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r))));
   const append_ms=await page.evaluate(()=>window.benchmark.append(1));
   await page.waitForTimeout(80);
   const layout=await page.evaluate(()=>{const s=document.querySelector('.conversation-scroll');const last=s.querySelector('.chat-turn:last-child');return {turns:s.querySelectorAll('.chat-turn').length,nodes:s.querySelectorAll('*').length,height:s.scrollHeight,bottom_gap:s.scrollHeight-s.scrollTop-s.clientHeight,last_bottom:last.getBoundingClientRect().bottom};});
   samples.push({render_ms,append_ms,layout});
  }
  rows.push({contain,initial_turns:count,samples});
 }
}
// All loaded text remains in DOM. Browser find must reveal an offscreen turn.
const find=await page.evaluate(()=>window.find('exact marker 420'));
await page.waitForTimeout(150);
const found=await page.evaluate(()=>{const s=document.querySelector('.conversation-scroll'); const t=s.querySelector('[data-turn-id="u-420"]');return {top:t.getBoundingClientRect().top,bottom:t.getBoundingClientRect().bottom,scrollTop:s.scrollTop};});
await page.locator('[data-turn-id="u-420"] .disclosure').click();
const openBefore=await page.locator('[data-turn-id="u-420"] [aria-label="Model thinking"]').innerText();
await page.getByRole('button',{name:'Jump to latest message'}).click();
await page.waitForTimeout(100);
await page.evaluate(()=>window.find('exact marker 420'));
await page.waitForTimeout(100);
const openAfter=await page.locator('[data-turn-id="u-420"] [aria-label="Model thinking"]').innerText();
const result={scope:'Same production NativeConversation fixture before/after injected CSS containment. Recreated fixture because the original temporary source was not retained; not directly comparable to the old run. GPU-disabled Chromium; no native WebKit claim.',rows,find,found,openBefore,openAfter,errors};
await fs.writeFile('implementation/evidence/overnight-resume-20260913/renderer-containment.json',JSON.stringify(result,null,2)+'\n');
await page.screenshot({path:'implementation/evidence/overnight-resume-20260913/renderer-containment.png'});
await browser.close();console.log(JSON.stringify(result));
if(errors.length||!find||openBefore!==openAfter)process.exitCode=1;
