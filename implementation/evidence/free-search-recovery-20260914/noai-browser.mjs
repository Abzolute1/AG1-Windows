import { chromium } from '../../reference/dyad-executable/node_modules/playwright/index.mjs';
import fs from 'node:fs';
const out='.local/free-search-recovery-20260914';
const browser=await chromium.launch({headless:true,executablePath:'/home/alex/.cache/ms-playwright/chromium-1234/chrome-linux64/chrome',args:['--disable-gpu']});
const report={requests:[]};
try {
 const page=await browser.newPage();page.setDefaultTimeout(15000);
 page.on('response',r=>{if(r.url().includes('duckduckgo.com')&&r.request().resourceType()!=='image')report.requests.push({url:r.url(),status:r.status()});});
 const response=await page.goto('https://noai.duckduckgo.com/?q=Rust+ownership+borrowing+documentation&ia=web',{waitUntil:'domcontentloaded'});
 report.status=response.status();report.headers=await response.allHeaders();
 try{await page.locator('[data-testid="result-title-a"], .anomaly-modal, #challenge-form').first().waitFor({timeout:12000});}catch{}
 report.results=await page.locator('[data-testid="result-title-a"]').evaluateAll(nodes=>nodes.map(n=>({title:n.textContent,url:n.href})));
 report.text=(await page.locator('body').innerText()).slice(0,12000);
 fs.writeFileSync(`${out}/browser-main-search.html`,await page.content());
} catch(e){report.error=String(e);} finally {
 fs.writeFileSync(`${out}/browser-main-probe.json`,JSON.stringify(report,null,2));
 console.log(JSON.stringify({status:report.status,results:report.results,text:report.text?.slice(0,1500),error:report.error},null,2));await browser.close();
}
