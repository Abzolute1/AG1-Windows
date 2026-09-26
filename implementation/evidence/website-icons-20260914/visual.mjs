import assert from 'node:assert/strict';
import fs from 'node:fs';
import {chromium} from '../../reference/dyad-executable/node_modules/playwright/index.mjs';
const out='implementation/evidence/website-icons-20260914';
const hosts=['openai.com','docs.python.org','github.com','news.ycombinator.com','duckduckgo.com','findskill.ai'];
const icons=Object.fromEntries(hosts.map(h=>[h,'data:image/png;base64,'+fs.readFileSync(`${out}/${h}.png`).toString('base64')]));
const browser=await chromium.launch({headless:true,executablePath:'/home/alex/.cache/ms-playwright/chromium_headless_shell-1234/chrome-headless-shell-linux64/chrome-headless-shell'});
const results=[];
try {
 for(const theme of ['dark','light']){
  const page=await browser.newPage({viewport:{width:650,height:800}});
  await page.addInitScript(({icons,theme})=>{window.probeIcons=icons;document.addEventListener('DOMContentLoaded',()=>{document.documentElement.classList.toggle('dark',theme==='dark');document.documentElement.dataset.theme=theme;});},{icons,theme});
  await page.route('**/icon-probe',route=>route.fulfill({contentType:'text/html',body:`<!doctype html><html><head><meta charset="utf-8"></head><body><div id="probe" style="padding:32px;max-width:600px"></div><script type="module">
import RefreshRuntime from '/@react-refresh';RefreshRuntime.injectIntoGlobalHook(window);window.$RefreshReg$=()=>{};window.$RefreshSig$=()=>(type)=>type;window.__vite_plugin_react_preamble_installed__=true;
const React=(await import('/node_modules/.vite/deps/react.js')).default;
const {createRoot}=(await import('/node_modules/.vite/deps/react-dom_client.js')).default;
const {SiteIcons,SourceList}=await import('/src/WebSources.tsx');
await import('/src/dyad.css');await import('/src/app.css');await import('/src/redesign.css');await import('/src/workflow.css');
const hosts=Object.keys(window.probeIcons),sources=hosts.map(host=>({entry:host,url:'https://'+host+'/',host,title:host}));
const run=async c=>({dataUrl:window.probeIcons[new URL(c.args.url).host]||null});
createRoot(document.getElementById('probe')).render(React.createElement(React.Fragment,null,
React.createElement('p',null,'Website icons from the native live check'),
React.createElement(SiteIcons,{sources,run}),React.createElement(SourceList,{sources,run}),
React.createElement('p',null,'Unavailable website — local fallback'),React.createElement(SiteIcons,{sources:[{entry:'none',url:'https://unavailable.example/',host:'unavailable.example',title:'Unavailable'}],run})));
</script></body></html>`}));
  const errors=[];page.on('pageerror',e=>{errors.push(e.message);console.log('PAGE ERROR',e.message);});
  await page.goto('http://127.0.0.1:1452/icon-probe');
  await page.waitForFunction(()=>document.querySelectorAll('.site-icon img').length===9,{},{timeout:10000}).catch(async e=>{console.log(await page.locator('body').innerText());console.log(await page.locator('.site-icon').count());throw e;});
  const checks=await page.locator('#probe').evaluate(el=>({images:[...el.querySelectorAll('img')].map(i=>({loaded:i.complete&&i.naturalWidth>0,width:i.width,height:i.height})),fallbacks:el.querySelectorAll('.site-icon svg').length,overflow:document.documentElement.scrollWidth>innerWidth}));
  assert(checks.images.every(i=>i.loaded&&i.width===16&&i.height===16));assert.equal(checks.fallbacks,1);assert(!checks.overflow);assert.deepEqual(errors,[]);
  await page.locator('#probe').screenshot({path:`${out}/${theme}-icons.png`});
  results.push({theme,...checks});await page.close();
 }
 fs.writeFileSync(`${out}/visual.json`,JSON.stringify({scope:'Private Chromium rendering of actual SiteIcons/SourceList with native-live PNG results; no user desktop capture or model inference',results},null,2));console.log('Both themes passed: all nine images decoded, fallback visible, no horizontal overflow.');
}finally{await browser.close();}
