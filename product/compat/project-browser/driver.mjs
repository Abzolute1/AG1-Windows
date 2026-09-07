// Optional, host-admitted automation. This helper owns no AgentSession or model.
import { createRequire } from 'node:module';
import { readFileSync } from 'node:fs';
const input = JSON.parse(readFileSync(0, 'utf8'));
const { chromium } = createRequire(import.meta.url)(input.playwright);
const allowed = (value) => {
  try { const u = new URL(value); return !u.username && !u.password && u.origin === input.origin; }
  catch { return false; }
};
if (!allowed(input.url)) throw new Error('URL outside admitted preview origin');
const browser = await chromium.launch({ executablePath: input.executable, headless: true, chromiumSandbox: true });
try {
  const context = await browser.newContext({ viewport: input.viewport ?? {width: 1024, height: 768}, serviceWorkers: 'block', acceptDownloads: false });
  const errors = [], consoleLines = [], blocked = [];
  await context.route('**/*', route => allowed(route.request().url()) ? route.continue() : ((blocked.length < 128 && blocked.push(route.request().url().slice(0,256))), route.abort()));
  await context.routeWebSocket('**/*', socket => {
    const url = new URL(socket.url());
    if (url.protocol === 'ws:') url.protocol = 'http:';
    if (allowed(url.href)) socket.connectToServer(); else socket.close();
  });
  context.on('page', page => {
    page.on('popup', popup => popup.close());
    page.on('dialog', dialog => dialog.dismiss());
    page.on('pageerror', error => { if (errors.length < 128) errors.push(String(error).slice(0,2048)); });
    page.on('console', message => { if (consoleLines.length < 128) consoleLines.push({type:message.type(),text:message.text().slice(0,2048)}); });
  });
  const page = await context.newPage();
  page.setDefaultTimeout(Math.min(input.timeoutMs ?? 15000,30000));
  await page.goto(input.url, {waitUntil:'networkidle'});
  for (const action of input.actions ?? []) {
    if (action.type === 'navigate') { if (!allowed(action.url)) throw new Error('Navigation outside admitted origin'); await page.goto(action.url); }
    else if (action.type === 'click') await page.locator(action.selector).click();
    else if (action.type === 'fill') await page.locator(action.selector).fill(action.value);
    else if (action.type === 'wait') await page.waitForTimeout(action.ms);
    else throw new Error('Unsupported visual action');
  }
  if (!allowed(page.url())) throw new Error('Preview navigated outside grant');
  const image = await page.screenshot({type:'png',fullPage:false});
  if (image.length > 8*1024*1024) throw new Error('Screenshot exceeds artifact budget');
  const result = {url:page.url(),title:await page.title(),text:(await page.locator('body').innerText()).slice(0,32768),errors,console:consoleLines,blocked,privileged:await page.evaluate(()=>({node:typeof process!=='undefined',tauri:typeof window.__TAURI_INTERNALS__!=='undefined'})),png:image.toString('base64')};
  process.stdout.write(JSON.stringify(result));
  await context.close();
} finally { await browser.close(); }
