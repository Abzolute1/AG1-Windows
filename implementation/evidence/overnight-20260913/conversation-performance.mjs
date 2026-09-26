import fs from 'node:fs/promises';
const {chromium}=await import('/tmp/ag1-github-visual/node_modules/playwright-core/index.mjs');
const browser=await chromium.launch({headless:true,executablePath:'/home/alex/.cache/ms-playwright/chromium-1234/chrome-linux64/chrome',args:['--disable-gpu']});
const page=await browser.newPage({viewport:{width:1600,height:1000}});const errors=[];page.on('pageerror',e=>errors.push(e.message));
await page.route('**/*',r=>r.request().url().startsWith('http://127.0.0.1:1432/')?r.continue():r.abort());
await page.goto('http://127.0.0.1:1432/conversation-performance.html');await page.waitForFunction(()=>!!window.benchmark);
const cdp=await page.context().newCDPSession(page);await cdp.send('Performance.enable');
const rows=[];
for(const count of [50,500,2000]){
 const samples=[];
 for(let trial=0;trial<3;trial++){
  const render_ms=await page.evaluate(n=>window.benchmark.reset(n),count);
  await page.evaluate(()=>new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r))));
  const append_ms=await page.evaluate(()=>window.benchmark.append(1));
  const layout=await page.evaluate(()=>{const s=document.querySelector('.conversation-scroll');const t=performance.now();s.scrollTop=0;const first=s.firstElementChild.getBoundingClientRect();s.scrollTop=s.scrollHeight;const last=s.querySelector('.chat-turn:last-child').getBoundingClientRect();return {ms:performance.now()-t,height:s.scrollHeight,first_top:first.top,last_bottom:last.bottom,turns:s.querySelectorAll('.chat-turn').length,nodes:s.querySelectorAll('*').length};});
  await cdp.send('HeapProfiler.collectGarbage');const metrics=Object.fromEntries((await cdp.send('Performance.getMetrics')).metrics.map(m=>[m.name,m.value]));
  samples.push({render_ms,append_ms,layout,js_heap_bytes:metrics.JSHeapUsedSize,dom_nodes:metrics.Nodes});
 }
 rows.push({initial_turns:count,samples});
}
await page.screenshot({path:'/home/alex/Projects/AG1/implementation/evidence/overnight-20260913/conversation-performance.png'});
const result={scope:'Real NativeConversation React component, synthetic local history, headless Chromium with GPU disabled; development build, three samples per size. Not a native WebKit or model latency measurement.',rows,errors};
await fs.writeFile('/home/alex/Projects/AG1/implementation/evidence/overnight-20260913/conversation-performance.json',JSON.stringify(result,null,2));
await browser.close();console.log(JSON.stringify(result));if(errors.length)process.exitCode=1;
