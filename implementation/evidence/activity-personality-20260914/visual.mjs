import assert from 'node:assert/strict';
import fs from 'node:fs';
import {chromium} from '../../reference/dyad-executable/node_modules/playwright/index.mjs';
const out='implementation/evidence/activity-personality-20260914';
const browser=await chromium.launch({headless:true,executablePath:'/home/alex/.cache/ms-playwright/chromium_headless_shell-1234/chrome-headless-shell-linux64/chrome-headless-shell'});
const records=[];
try {
 const page=await browser.newPage({viewport:{width:1080,height:1050}});
 const errors=[];page.on('pageerror',e=>errors.push(e.message));
 await page.goto('http://127.0.0.1:1453/activity-preview.html');
 await page.getByRole('heading',{name:'AG1 activity studio'}).waitFor();
 const status=page.locator('.activity-preview-grid .ag1-activity-status').first();
 await page.evaluate(()=>document.getAnimations().forEach(a=>{a.pause();a.currentTime=200;}));
 const sample=()=>status.evaluate(el=>({text:el.textContent,box:{width:el.querySelector('svg').getBoundingClientRect().width,height:el.querySelector('svg').getBoundingClientRect().height,x:el.querySelector('svg').getBoundingClientRect().x,y:el.querySelector('svg').getBoundingClientRect().y},paths:[...el.querySelectorAll('path')].map(p=>({opacity:getComputedStyle(p).opacity,transform:getComputedStyle(p).transform})),background:getComputedStyle(el.querySelector('.ag1-activity-word')).backgroundPosition}));
 const first=await sample();await status.screenshot({path:`${out}/motion-0200.png`});
 await page.evaluate(()=>document.getAnimations().forEach(a=>{a.pause();a.currentTime=1050;}));
 const next=await sample();await status.screenshot({path:`${out}/motion-1050.png`});
 assert.deepEqual(first.box,next.box);assert.equal(first.text,next.text);assert.notDeepEqual(first.paths,next.paths);assert.notEqual(first.background,next.background);
 await page.screenshot({path:`${out}/dark-studio.png`});
 await page.getByRole('button',{name:'Light theme',exact:true}).click();
 await page.screenshot({path:`${out}/light-studio.png`});
 const before=await page.locator('.activity-preview-grid').innerText();await page.getByRole('button',{name:'Next verbs'}).click();assert.notEqual(await page.locator('.activity-preview-grid').innerText(),before);
 for(const phase of ['Thinking','Generating','Waiting for model','Waiting for you','Stopping response…']){
  await page.getByRole('combobox',{name:'Conversation phase'}).selectOption(phase);
  const live=page.locator('.chat-turn > .ag1-activity-status');await live.waitFor();
  const location=await live.evaluate(el=>({last:el.parentElement.lastElementChild===el,label:el.getAttribute('aria-label'),text:el.textContent,paused:!!el.querySelector('[data-paused]')}));assert(location.last);
  if(phase==='Thinking')assert.equal(location.label,'Thinking');else if(phase==='Generating')assert.equal(location.label,'Generating response');else assert.equal(location.text,phase);
  if(phase==='Waiting for you')assert(location.paused);
  records.push({phase,...location});
 }
 await page.getByRole('combobox',{name:'Conversation phase'}).selectOption('Thinking');
 await page.getByRole('button',{name:/Working for 12s/}).click();
 assert(await page.locator('.chat-turn > .ag1-activity-status').isVisible());
 await page.setViewportSize({width:390,height:844});
 const narrow=await page.evaluate(()=>({scroll:document.documentElement.scrollWidth,viewport:innerWidth}));assert(narrow.scroll<=narrow.viewport);
 await page.screenshot({path:`${out}/narrow-studio.png`,fullPage:true});
 await page.emulateMedia({reducedMotion:'reduce'});
 const reduced=await status.evaluate(el=>({mark:[...el.querySelectorAll('path')].map(p=>getComputedStyle(p).animationName),word:getComputedStyle(el.querySelector('.ag1-activity-word')).animationName,fill:getComputedStyle(el.querySelector('.ag1-activity-word')).webkitTextFillColor,background:getComputedStyle(el.querySelector('.ag1-activity-word')).backgroundImage}));
 assert(reduced.mark.every(x=>x==='none'));assert.equal(reduced.word,'none');assert.equal(reduced.background,'none');assert(!reduced.fill.includes('rgba(0, 0, 0, 0)'));assert.deepEqual(errors,[]);
 fs.writeFileSync(`${out}/visual.json`,JSON.stringify({scope:'Private Chromium, real activity components and TurnView; simulated native phases; no user-screen capture or model runs',first,next,records,narrow,reduced,errors},null,2));
 console.log('Passed: moving highlight/strokes, fixed mark footprint and phrase, both themes, new variants, live bottom placement, literal states, collapsed log, narrow viewport and reduced motion.');
}finally{await browser.close();}
