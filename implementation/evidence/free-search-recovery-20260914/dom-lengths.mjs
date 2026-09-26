import {chromium} from '../../reference/dyad-executable/node_modules/playwright/index.mjs';
import fs from 'node:fs';
const b=await chromium.launch({headless:true,executablePath:'/home/alex/.cache/ms-playwright/chromium-1234/chrome-linux64/chrome',args:['--disable-gpu']});
try{const p=await b.newPage();const fragments=['<div><div></div><div></div','<br><div></br><br></div','<li><div></li><li></div','<p><div></p><p></div'];const results=await p.evaluate(f=>Object.fromEntries(f.map(s=>{const el=document.createElement('div');el.innerHTML=s;return[s,{html:el.innerHTML,length:el.innerHTML.length}]})),fragments);fs.writeFileSync('.local/free-search-recovery-20260914/dom-lengths.json',JSON.stringify(results,null,2));console.log(results);}finally{await b.close()}
