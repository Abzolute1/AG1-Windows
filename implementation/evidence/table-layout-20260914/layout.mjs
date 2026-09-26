import fs from 'node:fs';
import assert from 'node:assert/strict';
import React from '../../product/apps/desktop/node_modules/react/index.js';
import {renderToStaticMarkup} from '../../product/apps/desktop/node_modules/react-dom/server.node.js';
import Markdown from '../../product/apps/desktop/node_modules/react-markdown/index.js';
import gfm from '../../product/apps/desktop/node_modules/remark-gfm/index.js';
import {chromium} from '../../reference/dyad-executable/node_modules/playwright/index.mjs';
const out='implementation/evidence/table-layout-20260914';
const text=fs.readFileSync(`${out}/response.md`,'utf8');
const longToken='a'.repeat(240);
const extra=`\n\n| Identifier | Detail |\n|---|---|\n| ${longToken} | Long unbroken token |`;
const html=renderToStaticMarkup(React.createElement(Markdown,{remarkPlugins:[gfm]},text+extra));
const base=fs.readFileSync('product/apps/desktop/src/redesign.css','utf8');
const browser=await chromium.launch({headless:true,executablePath:'/home/alex/.cache/ms-playwright/chromium_headless_shell-1234/chrome-headless-shell-linux64/chrome-headless-shell'});
const page=await browser.newPage();
const results=[];
try {
 for(const stage of ['before','after']) {
  const css=fs.readFileSync(stage==='before'?'.local/table-layout-20260914/markdown-before.css':'product/apps/desktop/src/markdown.css','utf8');
  for(const width of [1150,768,390]) {
   await page.setViewportSize({width,height:1000});
   await page.setContent(`<style>${base}\n${css}\n:root{--chat-font-size:20px;--foreground:#ededed;--background:#19191c;--border:#36363b}body{margin:0;background:var(--background);color:var(--foreground);font-family:Arial}*{box-sizing:border-box}</style><article class="chat-turn"><div class="markdown">${html}</div></article>`);
   const metrics=await page.evaluate(()=> {
    const tables=[...document.querySelectorAll('table')];
    const support=tables.find(t=>t.textContent.includes('KTS100 MK8')&&t.textContent.includes("Why It's Strong"));
    const summary=tables.find(t=>t.textContent.includes('Runner-Up'));
    const cell=t=>[...t.rows[0].cells].map(c=>({text:c.textContent,width:c.getBoundingClientRect().width,height:c.getBoundingClientRect().height,wrap:getComputedStyle(c).overflowWrap}));
    // Measure individual glyph positions: words must stay on one line.
    const intact=(el,word)=>{
     const walker=document.createTreeWalker(el,NodeFilter.SHOW_TEXT);let n;
     while(n=walker.nextNode()) {const i=n.textContent.indexOf(word);if(i<0)continue;
      const ys=[];for(let j=0;j<word.length;j++){const r=document.createRange();r.setStart(n,i+j);r.setEnd(n,i+j+1);ys.push(r.getBoundingClientRect().y)}return new Set(ys).size===1;
     }return false;
    };
    return {support:cell(support),summary:cell(summary),rankIntact:intact(support,'Rank'),weaponIntact:intact(support,'KTS100'),pageOverflow:document.documentElement.scrollWidth>innerWidth,longTokenScroll:tables.at(-1).scrollWidth>tables.at(-1).clientWidth};
   });
   results.push({stage,width,...metrics});
   if(stage==='after'){assert(metrics.rankIntact&&metrics.weaponIntact,JSON.stringify(metrics));assert(!metrics.pageOverflow);assert(metrics.longTokenScroll);}
   if(width===1150) {
    await page.locator('h3').filter({hasText:'Support Class'}).scrollIntoViewIfNeeded();
    await page.screenshot({path:`${out}/${stage}.png`});
   }
  }
 }
 assert(results.some(r=>r.stage==='before'&&!r.rankIntact),'must reproduce regression');
 fs.writeFileSync(`${out}/layout.json`,JSON.stringify(results,null,2));
 console.log(JSON.stringify({status:'passed',cases:results.length,results},null,2));
}finally{await browser.close()}
