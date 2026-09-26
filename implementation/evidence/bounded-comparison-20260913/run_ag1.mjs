import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import {runNativeFixture} from './native-harness.mjs';
const root='/home/alex/Projects/AG1', evidence=path.dirname(new URL(import.meta.url).pathname);
const candidate=root+'/.local/p14-live-fixes/native-candidate-compare13';
const plan=JSON.parse(fs.readFileSync(evidence+'/plan.json'));
const config=JSON.parse(fs.readFileSync(root+'/.local/p14-long-horizon/scenario-validation-20260913/candidate/config.json'));
config.max_generations=32;
config.system_prompt='You are a coding and research assistant. Complete the requested tasks with the available tools and report actual results accurately.';
config.tooling.process_host=candidate+'/ag1-process-host';
const pause=ms=>new Promise(r=>setTimeout(r,ms));
await runNativeFixture(candidate, {prefix:'cmp-', config,
 async beforeStart({work}) {
  fs.writeFileSync(evidence+'/ag1-work.txt',work+'\n');
  const settings=path.join(work,'settings.json');fs.copyFileSync(config.imported.settings.path,settings);
  const cfg={...config,imported:{...config.imported,settings:{path:settings,sha256:crypto.createHash('sha256').update(fs.readFileSync(settings)).digest('hex')}}};
  fs.writeFileSync(path.join(work,'host.json'),JSON.stringify(cfg),{mode:0o600});
  fs.mkdirSync(path.join(work,'web'));fs.cpSync(root+'/.local/bounded-comparison-20260913/seed',path.join(work,'coding'),{recursive:true});
 },
 async exercise({work,probe,wait}) {
  const js=script=>probe({script}), native=async()=>(await probe({native_view:true})).view;
  const click=label=>wait(()=>js(`(()=>{const b=[...document.querySelectorAll('button')].find(b=>b.getClientRects().length&&(b.getAttribute('aria-label')===${JSON.stringify(label)}||b.textContent.trim()===${JSON.stringify(label)}));if(!b||b.disabled)return false;b.click();return true})()`),'click '+label);
  const fill=(selector,value,type='HTMLInputElement')=>js(`(()=>{const e=document.querySelector(${JSON.stringify(selector)});if(!e)return false;Object.getOwnPropertyDescriptor(${type}.prototype,'value').set.call(e,${JSON.stringify(value)});e.dispatchEvent(new Event('input',{bubbles:true}));return true})()`);
  const send=async command=>{
   const v=await native(),envelope={version:v.snapshot.version,scope:v.snapshot.scope,expected_revision:v.snapshot.revision,id:'compare-'+crypto.randomUUID(),command};
   await js(`window.__cmpReply=null;window.__TAURI_INTERNALS__.invoke('bridge_command',${JSON.stringify({envelope})}).then(reply=>window.__cmpReply={reply},error=>window.__cmpReply={error:String(error)});true`);
   await wait(()=>js('window.__cmpReply!==null'),'native reply');return js('window.__cmpReply');
  };
  async function openProject(name) {
   await js(`document.dispatchEvent(new KeyboardEvent('keydown',{key:'o',ctrlKey:true,bubbles:true}));true`);
   await wait(()=>js(`!!document.querySelector('input[aria-label="Project folder"]')`),'project dialog');
   await fill('input[aria-label="Project folder"]',path.join(work,name));await click('Open project');
   await wait(()=>js(`!document.querySelector('input[aria-label="Project folder"]')`),'project open');
  }
  await click('Back to chat');const stages=[];
  for(const task of plan.cases) {
   if(task.id!=='followup')await openProject(task.id);
   if(task.id==='web')await send({kind:'runtime',action:'tools',args:{names:['webSearch','webFetch']}});
   const output=path.join(evidence,'ag1-'+task.id);fs.mkdirSync(output);
   const before=await native();fs.writeFileSync(output+'/before.json',JSON.stringify(before,null,2));
   await fill('textarea[aria-label="Message AG1"]',task.prompt,'HTMLTextAreaElement');await click('Send message');
   const began=Date.now(),frames=[],shots=new Set(),approved=new Set();let activeSeen=false,done=false;
   while(Date.now()-began<task.limit_seconds*1000) {
    const v=await native();const active=['running','streaming','awaiting_input'].includes(v.snapshot.status);activeSeen ||= active;
    const dom=await js(`(()=>{const turn=document.querySelector('.chat-turn:last-of-type'),thinking=[...(turn?.querySelectorAll('.disclosure')||[])].filter(b=>b.textContent.trim()==='Thinking'),lastTool=[...(turn?.querySelectorAll('.tool-entry-toggle')||[])].at(-1),status=turn?.querySelector('.work-phase');return {text:document.body.innerText,thinking:thinking.length,thinkingBelowTools:thinking.every(b=>!lastTool||!!(lastTool.compareDocumentPosition(b)&Node.DOCUMENT_POSITION_FOLLOWING)),statusBelowTools:!status||!lastTool||!!(lastTool.compareDocumentPosition(status)&Node.DOCUMENT_POSITION_FOLLOWING),icons:[...document.querySelectorAll('.site-icon img')].map(i=>({loaded:i.complete&&i.naturalWidth>0,host:i.parentElement.title})),tools:turn?.querySelectorAll('.tool-entry-toggle').length||0,innerScroll:[...document.querySelectorAll('.response-work.is-working .work-log')].some(e=>getComputedStyle(e).overflowY==='auto'),pageOverflow:document.documentElement.scrollWidth>innerWidth+1}})()`);
    const frame={ms:Date.now()-began,status:v.snapshot.status,pending:v.snapshot.pending,...dom};frames.push(frame);
    fs.writeFileSync(output+'/latest.json',JSON.stringify({elapsed:frame.ms,status:v.snapshot.status,tools:dom.tools,icons:dom.icons,tail:dom.text.slice(-2000)},null,2));
    const tag=dom.icons.some(i=>i.loaded)&&dom.thinking?'thinking-with-icons':dom.icons.some(i=>i.loaded)?'icons-visible':dom.thinking&&dom.tools?'thinking-after-tools':dom.thinking?'initial-thinking':dom.tools?'tools-active':null;
    if(tag&&!shots.has(tag)){await probe({screenshot:task.id+'-'+tag+'.png'});shots.add(tag);}
    for(const pending of v.snapshot.pending||[]) {
     if(pending.binding&&!approved.has(JSON.stringify(pending.binding))){approved.add(JSON.stringify(pending.binding));const reply=await send({kind:'approve',binding:pending.binding,allow:true});fs.appendFileSync(output+'/approvals.jsonl',JSON.stringify({pending,reply})+'\n');}
    }
    if(activeSeen&&!active&&!v.snapshot.pending.length){done=true;break;}
    await pause(750);
   }
   if(!done){await send({kind:'cancel'}).catch(()=>{});await click('Stop response').catch(()=>{});await pause(1000);}
   const final=await native();fs.writeFileSync(output+'/final-view.json',JSON.stringify(final,null,2));fs.writeFileSync(output+'/frames.json',JSON.stringify(frames,null,2));
   if(done)await wait(()=>js('!document.querySelector(".response-work.is-working")'),'renderer settled');
   await probe({screenshot:task.id+'-completed.png'});
   const stage={id:task.id,completed:done,seconds:(Date.now()-began)/1000,shots:[...shots],frames:frames.length,approvalCount:approved.size,status:final.snapshot.status};stages.push(stage);fs.writeFileSync(output+'/result.json',JSON.stringify(stage,null,2));console.log(JSON.stringify(stage));
   if(task.id==='web') {
    await js(`(()=>{for(const b of document.querySelectorAll('.work-toggle'))if(b.getAttribute('aria-expanded')==='false')b.click();return true})()`);await pause(300);
    await js(`(()=>{for(const b of document.querySelectorAll('.tool-entry-toggle'))if(b.querySelector('.site-icons')&&b.getAttribute('aria-expanded')==='false'){b.click();break;}return true})()`);await pause(1200);
    for(const [width,height]of [[1440,900],[900,600]]){await probe({size:[width,height]});await wait(()=>js(`innerWidth===${width}`),'resize');await js(`document.querySelector('.web-sources')?.scrollIntoView({block:'center'});true`);await pause(250);await probe({screenshot:'web-sources-'+width+'.png'});}
    await probe({size:[1440,900]});
    fs.writeFileSync(output+'/source-dom.json',JSON.stringify(await js(`({icons:[...document.querySelectorAll('.site-icon img')].map(i=>({host:i.parentElement.title,loaded:i.complete&&i.naturalWidth>0})),sources:[...document.querySelectorAll('.web-source')].map(a=>({text:a.innerText,url:a.href}))})`),null,2));
   } else {fs.cpSync(path.join(work,'coding'),path.join(output,'workspace'),{recursive:true});}
   if(!done)break;
  }
  fs.writeFileSync(evidence+'/ag1-stages.json',JSON.stringify(stages,null,2));
  return {promptsSubmitted:stages.length,stages,actualLocalModel:true,model:config.selected};
 }
});
