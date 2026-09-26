import { chromium } from '../../reference/dyad-executable/node_modules/playwright/index.mjs';
import fs from 'node:fs';
const out = '.local/duckduckgo-request-flow-20260914';
const browser = await chromium.launch({headless:true, executablePath:'/home/alex/.cache/ms-playwright/chromium-1234/chrome-linux64/chrome', args:['--disable-gpu']});
const reports=[];
try {
 const page=await browser.newPage();
 page.setDefaultTimeout(15000);
 const initial=await page.goto('https://html.duckduckgo.com/html/',{waitUntil:'domcontentloaded'});
 reports.push({stage:'landing',status:initial.status(),headers:await initial.allHeaders()});
 await page.locator('input[name=q]').fill('site:urllib3.readthedocs.io Retry allowed_methods');
 const [response]=await Promise.all([page.waitForNavigation({waitUntil:'domcontentloaded'}),page.locator('input[name=q]').press('Enter')]);
 const html=await page.content();fs.writeFileSync(`${out}/browser-search.html`,html);
 const results=await page.locator('.result__a').evaluateAll(nodes=>nodes.map(n=>({title:n.textContent,url:n.href})));
 reports.push({stage:'search',status:response.status(),url:page.url(),headers:await response.allHeaders(),challenge:/anomaly-modal|challenge-form/.test(html),results});
} catch(e) {reports.push({error:String(e)});} finally {
 fs.writeFileSync(`${out}/browser-probe.json`,JSON.stringify(reports,null,2));
 console.log(JSON.stringify(reports.map(({headers,...r})=>r),null,2));await browser.close();
}
