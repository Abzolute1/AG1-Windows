// Deterministic browser layout check. All desktop calls are in-memory fixtures;
// this script has no native bridge and cannot invoke a model or project tools.
import assert from 'node:assert/strict';
const { chromium } = await import(process.env.AG1_PLAYWRIGHT_MODULE || 'playwright');
const browser = await chromium.launch({ headless: true });
try {
  const page = await browser.newPage({ viewport: { width: 2476, height: 1356 } });
  await page.addInitScript(() => {
    const snapshot = { version: {major:1,minor:0,schema:'fixture'}, scope:{workspace:'w',session:'s',instance:'i',epoch:'1'},revision:'1',watermark:'1',workspace_path:'/fixture',status:'idle',active_operation:null,messages:[],message_count:0,pending:[],model:'fixture',diagnostic:null,session_file:null,fixture_provider:true };
    window.__audit = { files:['notes.txt'], commands:[] };
    window.__TAURI_INTERNALS__ = { transformCallback:()=>1, unregisterCallback:()=>{}, invoke:async (name,args) => {
      if (name==='catalog') return {projects:['/fixture'],theme:'dark',chats:[]};
      if (name==='desktop_preferences') return {approval_mode:'interactive'};
      if (name==='desktop_models') return [];
      if (name==='bridge_view') return {snapshot,error:null,progress:''};
      if (name==='bridge_command') {
        const command=args.envelope.command;
        window.__audit.commands.push(command);
        if (command.kind!=='files') throw new Error('Unexpected non-file command '+command.kind);
        let data;
        if (command.action==='list') data=[...window.__audit.files];
        else if (command.action==='read') data={path:command.args.path,text:'original',revision:'r1'};
        else throw new Error('Unexpected mutation '+command.action);
        return {kind:'reply',id:args.envelope.id,scope:snapshot.scope,revision:'1',durable:false,status:'query',data};
      }
      if (name.startsWith('plugin:event|')) return 1;
      throw new Error('Unexpected bridge call '+name);
    }};
  });
  await page.goto('http://127.0.0.1:1429');
  await page.getByRole('button',{name:'Files',exact:true}).click();
  await page.getByRole('button',{name:'notes.txt',exact:true}).click();
  await page.locator('.monaco-editor').waitFor();
  const measurements=[];
  for (const size of [{width:2476,height:1356},{width:1000,height:700},{width:640,height:480}]) {
    await page.setViewportSize(size);
    if (size.width < 760) await page.getByRole('button',{name:'Hide sidebar'}).click();
    await page.waitForFunction(() => {const editor=document.querySelector('.monaco-editor'), pane=document.querySelector('.file-pane'), toolbar=document.querySelector('.file-toolbar'); if (!editor || !pane || !toolbar) return false; const e=editor.getBoundingClientRect(),p=pane.getBoundingClientRect(),t=toolbar.getBoundingClientRect(); return Math.abs(e.width-p.width)<4 && Math.abs(e.height-(p.height-t.height))<5;});
    const dims=await page.evaluate(()=> {
      const box=s=> {const r=document.querySelector(s).getBoundingClientRect();return {width:r.width,height:r.height};};
      return {editor:box('.monaco-editor'),pane:box('.file-pane'),toolbar:box('.file-toolbar')};
    });
    assert.ok(Math.abs(dims.editor.height-(dims.pane.height-dims.toolbar.height))<5,JSON.stringify(dims));
    measurements.push({viewport:size,...dims});
  }
  await page.locator('.monaco-editor textarea').focus();
  await page.keyboard.press('Control+a');
  await page.keyboard.insertText('unsaved local edit');
  await page.getByLabel('Unsaved changes').waitFor();
  const before = (await page.locator('.view-lines').innerText()).replace(/\s+/g,' ');
  await page.evaluate(()=>window.__audit.files.push('created.txt'));
  await page.getByRole('button',{name:'Refresh files'}).click();
  await page.getByRole('button',{name:'created.txt',exact:true}).waitFor();
  await page.getByLabel('Unsaved changes').waitFor();
  assert.equal((await page.locator('.view-lines').innerText()).replace(/\s+/g,' '),before);
  assert.ok(before.includes('unsaved local edit'));
  await page.screenshot({path:'.local/p14-live-fixes/resumed-editor-browser.png'});
  console.log(JSON.stringify({measurements,unsavedRefreshPreserved:true,nativeBridge:false,inference:false},null,2));
} finally { await browser.close(); }
