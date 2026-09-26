import assert from 'node:assert/strict';
import fs from 'node:fs';
import {chromium} from '../../reference/dyad-executable/node_modules/playwright/index.mjs';
const out='implementation/evidence/model-loading-20260914';
const browser=await chromium.launch({headless:true,executablePath:'/home/alex/.cache/ms-playwright/chromium_headless_shell-1234/chrome-headless-shell-linux64/chrome-headless-shell'});
const results=[];
try {
 for(const theme of ['dark','light']){
  const page=await browser.newPage({viewport:{width:1280,height:850}});
  await page.addInitScript(theme=>{
   let active=false, progress=null, finish;
   const snapshot={version:{major:1,minor:0,schema:'fixture'},scope:{workspace:'ws',session:'s',instance:'i',epoch:'1'},revision:'1',watermark:'1',workspace_path:'/private/general-chat',status:'idle',active_operation:null,messages:[],message_count:0,pending:[],model:'Qwen3.6-35B-A3B',diagnostic:null,session_file:null,fixture_provider:true};
   const view=()=>({snapshot:active?snapshot:null,error:null,progress:'',...(active?{details:{authority:'rust-isolated',messages:[],task:null,context:{selected_model:{provider:'llamacpp',id:snapshot.model}},memory:null,artifacts:[],activity:{runtime_preparation:progress},models:[],capabilities:[]}}:{})});
   window.__modelProbe={commands:[],set:(phase,percent)=>{progress={model:snapshot.model,phase,percent,started_at:Date.now()};},finish:()=>{progress={model:snapshot.model,phase:'ready',started_at:Date.now()};finish?.();}};
   window.__TAURI_INTERNALS__={transformCallback:()=>1,unregisterCallback:()=>{},invoke:async(name,args)=>{
    if(name==='catalog')return {projects:[],chats:[],theme};
    if(name==='desktop_preferences')return {approval_mode:'interactive'};
    if(name==='desktop_models')return [{id:snapshot.model,provider:'llamacpp'}];
    if(name==='open_general'){active=true;return view();}
    if(name==='bridge_view')return view();
    if(name==='conversation_statuses')return [];
    if(name==='bridge_command'){
     const c=args.envelope.command;window.__modelProbe.commands.push(c);
     if(c.kind==='prompt')throw Error('Manual loading sent a prompt');
     if(c.action==='load_model'){progress={model:snapshot.model,phase:'verifying',percent:63,started_at:Date.now()};await new Promise(r=>finish=r);}
     let data=c.action==='catalog'?{chats:[],next:null}:c.action==='extension_settings'?{settings:[]}:null;
     return {kind:'reply',status:'completed',data};
    }
    if(name.startsWith('plugin:event|'))return 1;
    return null;
   }};
  },theme);
  await page.goto('http://127.0.0.1:1446');
  const input=page.getByRole('textbox',{name:'Message AG1'});
  await input.fill('A draft I can keep writing while the model loads.');
  await page.getByRole('button',{name:'Load',exact:true}).waitFor();
  await page.locator('.composer').screenshot({path:`${out}/${theme}-load.png`});
  await page.getByRole('button',{name:'Load',exact:true}).click();
  await page.getByRole('progressbar',{name:'Checking model files'}).waitFor();
  assert.equal(await page.getByRole('progressbar').getAttribute('aria-valuenow'),'63');
  await input.fill('A draft I kept writing during loading.');
  assert.equal(await page.getByText('Recovering the stopped response…').count(),0);
  await page.locator('.composer').screenshot({path:`${out}/${theme}-progress.png`});
  await page.evaluate(()=>window.__modelProbe.set('loading'));
  await page.getByRole('status').filter({hasText:'Loading model'}).waitFor();
  assert.equal(await page.getByRole('progressbar').count(),0);
  await page.evaluate(()=>window.__modelProbe.finish());
  await page.getByRole('status').filter({hasText:'Loaded'}).waitFor();
  assert.equal(await input.inputValue(),'A draft I kept writing during loading.');
  await page.locator('.composer').screenshot({path:`${out}/${theme}-loaded.png`});
  const commands=await page.evaluate(()=>window.__modelProbe.commands);
  assert.equal(commands.filter(c=>c.action==='load_model').length,1);assert(!commands.some(c=>c.kind==='prompt'));
  await page.setViewportSize({width:560,height:800});
  const hide=page.getByRole('button',{name:'Hide sidebar',exact:true});if(await hide.count())await hide.click();
  await page.evaluate(()=>window.__modelProbe.set('verifying',87));
  await page.getByRole('progressbar').waitFor();
  await page.locator('.composer').screenshot({path:`${out}/${theme}-narrow.png`});
  const layout=await page.locator('.model-load-control').evaluate(el=>{const r=el.getBoundingClientRect();return {left:r.left,right:r.right,viewport:innerWidth,scroll:el.scrollWidth,client:el.clientWidth,color:getComputedStyle(el).color};});
  assert(layout.left>=0&&layout.right<=layout.viewport);assert(layout.scroll<=layout.client+1);
  results.push({theme,draftPreserved:true,noPrompt:true,measuredPercent:true,layout});await page.close();
 }
 fs.writeFileSync(`${out}/visual.json`,JSON.stringify(results,null,2));console.log(results);
}finally{await browser.close();}
