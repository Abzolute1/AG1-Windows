import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import assert from 'node:assert/strict';
import {runNativeFixture} from './native-harness.mjs';
const root='/home/alex/Projects/AG1', candidate=root+'/.local/p14-live-fixes/native-candidate-chat13-final';
const config=JSON.parse(fs.readFileSync(root+'/.local/p14-long-horizon/scenario-validation-20260913/candidate/config.json','utf8'));
config.max_generations=12;config.system_prompt='You are AG1 testing an isolated read-only web research workflow. Use webSearch and webFetch, give short progress updates, and report observed results accurately. Do not modify files.';
config.tooling.process_host=candidate+'/ag1-process-host';
const start=Date.now(), elapsed=()=>Date.now()-start, pause=ms=>new Promise(r=>setTimeout(r,ms));
await runNativeFixture(candidate,{
 prefix:'web2-',config,
 async beforeStart({work}) {
  const settings=path.join(work,'settings.json');fs.copyFileSync(config.imported.settings.path,settings);
  const cfg={...config,imported:{...config.imported,settings:{path:settings,sha256:crypto.createHash('sha256').update(fs.readFileSync(settings)).digest('hex')}}};
  fs.writeFileSync(path.join(work,'host.json'),JSON.stringify(cfg),{mode:0o600});
 },
 async exercise({work,probe,wait}) {
  const js=script=>probe({script});const native=async()=>(await probe({native_view:true})).view;
  const click=async label=>wait(()=>js(`(()=>{const b=[...document.querySelectorAll('button')].find(b=>b.getAttribute('aria-label')===${JSON.stringify(label)}||b.textContent.trim()===${JSON.stringify(label)});if(!b||b.disabled)return false;b.click();return true})()`),'click '+label);
  const send=async command=>{
   const v=await native(),envelope={version:v.snapshot.version,scope:v.snapshot.scope,expected_revision:v.snapshot.revision,id:'visual-'+crypto.randomUUID(),command};
   await js(`window.__chatReply=null;window.__TAURI_INTERNALS__.invoke('bridge_command',${JSON.stringify({envelope})}).then(reply=>window.__chatReply={reply},error=>window.__chatReply={error:String(error)});true`);
   await wait(()=>js('window.__chatReply!==null'),'native action reply');const r=await js('window.__chatReply');assert.ok(!r.error,JSON.stringify(r));assert.notEqual(r.reply?.status,'rejected',JSON.stringify(r));return r;
  };
  await click('Back to chat');
  const project=path.join(work,'project');fs.mkdirSync(project,{mode:0o700});
  await js(`document.dispatchEvent(new KeyboardEvent('keydown',{key:'o',ctrlKey:true,bubbles:true}));true`);
  await wait(()=>js(`!!document.querySelector('input[aria-label="Project folder"]')`),'project dialog');
  await js(`(()=>{const e=document.querySelector('input[aria-label="Project folder"]');Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(e,${JSON.stringify(project)});e.dispatchEvent(new Event('input',{bubbles:true}));return true})()`);
  await click('Open project');
  await wait(()=>js(`!document.querySelector('input[aria-label="Project folder"]')`),'project opened');
  await send({kind:'runtime',action:'tools',args:{names:['webSearch','webFetch']}});
  const prompt='Search the web once for "GitHub" with maxResults 3. Then fetch https://github.com/ regardless of whether the search found results. Give one brief progress update before each tool call, and finish with two sentences explaining what GitHub is with a source link. Do not retry an empty search. Do not create or edit files.';
  await js(`(()=>{const e=document.querySelector('textarea[aria-label="Message AG1"]');Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,'value').set.call(e,${JSON.stringify(prompt)});e.dispatchEvent(new Event('input',{bubbles:true}));return true})()`);
  await click('Send message');
  const began=Date.now(),shots=[],observations=[];let activeSeen=false,stopped=false;
  while(Date.now()-began<240000 && elapsed()<900000) {
   const v=await native();const active=['running','streaming','awaiting_input'].includes(v.snapshot.status);activeSeen ||= active;
   const dom=await js(`(()=>{const turn=document.querySelector('.chat-turn:last-of-type');const buttons=[...document.querySelectorAll('.disclosure')];const thinking=buttons.filter(b=>b.textContent.trim()==='Thinking');const lastTool=[...document.querySelectorAll('.tool-entry-toggle')].at(-1);const status=document.querySelector('.work-phase');const scroller=document.querySelector('.conversation-scroll');return {text:document.body.innerText,thinking:thinking.length,thinkingBelowTools:thinking.every(b=>!lastTool||!!(lastTool.compareDocumentPosition(b)&Node.DOCUMENT_POSITION_FOLLOWING)),statusBelowTools:!status||!lastTool||!!(lastTool.compareDocumentPosition(status)&Node.DOCUMENT_POSITION_FOLLOWING),icons:document.querySelectorAll('.site-icon img').length,sources:document.querySelectorAll('.site-icons').length,innerScroll:[...document.querySelectorAll('.response-work.is-working .work-log')].some(e=>getComputedStyle(e).overflowY==='auto'),gap:scroller?scroller.scrollHeight-scroller.scrollTop-scroller.clientHeight:null}})()`);
   observations.push({ms:Date.now()-began,status:v.snapshot.status,...dom});
   assert.ok(dom.thinkingBelowTools && dom.statusBelowTools && !dom.innerScroll,'Active work chronology');
   let tag=dom.thinking && dom.sources?'thinking-after-search':dom.sources && dom.icons?'search-icons':dom.thinking?'initial-thinking':null;
   if(tag&&!shots.includes(tag)) {await probe({screenshot:tag+'.png'});shots.push(tag);fs.writeFileSync(path.join(work,tag+'-view.json'),JSON.stringify(v,null,2));}
   if(activeSeen&&!active&&!v.snapshot.pending.length){stopped=true;break;}
   await pause(400);
  }
  if(!stopped){await click('Stop response').catch(()=>{});throw Error('Bounded native live prompt did not complete; stopped at deadline');}
  await wait(()=>js('!document.querySelector(".response-work.is-working")'),'renderer settled');
  // Expand the completed work and a real search result for source-card inspection.
  await js(`(()=>{const w=document.querySelector('.work-toggle');if(w?.getAttribute('aria-expanded')==='false')w.click();return true})()`);
  await wait(()=>js('!!document.querySelector(".tool-entry-toggle")'),'retained work rows');
  await js(`(()=>{const b=[...document.querySelectorAll('.tool-entry-toggle')].find(b=>b.querySelector('.site-icons'));if(b?.getAttribute('aria-expanded')==='false')b.click();return true})()`);
  await wait(()=>js('!!document.querySelector(".web-source")'),'expanded native sources');
  await js(`document.querySelector('.web-sources').scrollIntoView({block:'center'});true`);
  await pause(1200);
  await probe({screenshot:'completed-web-research.png'});
  const final=await native();fs.writeFileSync(path.join(work,'final-view.json'),JSON.stringify(final,null,2));fs.writeFileSync(path.join(work,'observations.json'),JSON.stringify(observations,null,2));
  const finalDom=await js('({text:document.body.innerText,icons:document.querySelectorAll(".site-icon img").length,links:[...document.querySelectorAll(".web-source")].map(a=>({text:a.innerText,href:a.href})),thinking:[...document.querySelectorAll(".disclosure")].filter(e=>e.textContent.trim()==="Thinking").length})');
  fs.writeFileSync(path.join(work,'final-dom.json'),JSON.stringify(finalDom,null,2));assert.equal(finalDom.thinking,0);
  assert.ok(finalDom.icons > 0 && finalDom.links.length > 0,'Native website icon and source link visible');
  const sizes=[];
  for(const [width,height] of [[1440,900],[900,600]]) {
   await probe({size:[width,height]});await wait(()=>js(`innerWidth===${width}&&innerHeight===${height}`),'window resized');
   await js(`document.querySelector('.web-sources').scrollIntoView({block:'center'});true`);await pause(300);
   const layout=await js(`({width:innerWidth,scrollWidth:document.documentElement.scrollWidth,color:getComputedStyle(document.querySelector('.web-source-count')).color,sourceWidth:document.querySelector('.web-source').getBoundingClientRect().width})`);assert.ok(layout.scrollWidth<=width+1);sizes.push(layout);
   await probe({screenshot:'sources-'+width+'.png'});
  }
  return {sizes,promptsSubmitted:1,model:config.selected,seconds:(Date.now()-began)/1000,shots,observedFrames:observations.length,finalDom,actualLocalModel:true,screen:'private authenticated Xvfb only'};
 }
});
