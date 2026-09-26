// Owned native recovery: a later folder collision interrupts a real approved
// checkout. No forged private records, injected product hooks or live GitHub.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { runNativeFixture } from './p14_native_startup_smoke.mjs';
let generations=0,targetSha='',recoveryId='';const errors=[];
function handleRequest(request,response) {
  let body='';request.on('data',part=>{body+=part;if(body.length>4*1024*1024)request.destroy();});
  request.on('end',()=>{try {
    const data=JSON.parse(body);assert.ok(generations<4);assert.ok(data.tools.some(t=>t.function?.name==='github'));
    const turn=generations++,repair=turn>=2,first=turn%2===0,id=repair?'owned-repair-call':'owned-interrupted-checkout-call';
    const args=repair?{operation:'git.recover',repo:'owned/fixture',parameters:{recovery_id:recoveryId,choice:'finish'}}:{operation:'git.checkout',repo:'owned/fixture',parameters:{branch:'model-recovery-target',target_sha:targetSha}};
    if(!first) {const result=data.messages.find(m=>m.role==='tool'&&m.tool_call_id===id);assert.ok(result);assert.ok(JSON.stringify(result).includes(repair?'applied':'unknown'));}
    const delta=first?{tool_calls:[{index:0,id,type:'function',function:{name:'github',arguments:JSON.stringify(args)}}]}:{content:repair?'Owned native recovery finished; retained originals remain.':'Owned checkout interrupted; no automatic retry.'};
    response.writeHead(200,{'Content-Type':'text/event-stream',Connection:'close'});response.end(`data: ${JSON.stringify({choices:[{delta,finish_reason:first?'tool_calls':'stop'}],usage:{prompt_tokens:100,completion_tokens:35}})}\n\ndata: [DONE]\n\n`);
  }catch(error){errors.push(String(error));response.writeHead(400);response.end('{}');}});
}
await runNativeFixture(process.argv[2],{prefix:'quota-',githubFixture:true,gitReview:true,handleRequest,
  async exercise({work,probe,wait,requests}) {
    const js=script=>probe({script});
    const click=label=>wait(()=>js(`(()=>{const b=[...document.querySelectorAll('button')].find(b=>b.getClientRects().length&&(b.getAttribute('aria-label')===${JSON.stringify(label)}||b.textContent.trim()===${JSON.stringify(label)}));if(!b||b.disabled)return false;b.focus();b.click();return true})()`),`click ${label}`);
    const fill=(selector,value)=>js(`(()=>{const e=document.querySelector(${JSON.stringify(selector)});if(!e)return false;Object.getOwnPropertyDescriptor(e.tagName==='TEXTAREA'?HTMLTextAreaElement.prototype:HTMLInputElement.prototype,'value').set.call(e,${JSON.stringify(value)});e.dispatchEvent(new Event('input',{bubbles:true}));return true})()`);
    const select=(selector,value)=>js(`(()=>{const e=document.querySelector(${JSON.stringify(selector)});if(!e)return false;e.value=${JSON.stringify(value)};e.dispatchEvent(new Event('change',{bubbles:true}));return true})()`);
    await click('Back to chat');
    const project=path.join(work,'project'),main=path.join(work,'main');fs.mkdirSync(main,{mode:0o700});
    const mainGit=args=>execFileSync('/usr/bin/git',args,{cwd:main,env:{PATH:'/usr/bin:/bin',GIT_CONFIG_NOSYSTEM:'1',GIT_CONFIG_GLOBAL:'/dev/null',GIT_AUTHOR_NAME:'Owned',GIT_COMMITTER_NAME:'Owned',GIT_AUTHOR_EMAIL:'owned@example.invalid',GIT_COMMITTER_EMAIL:'owned@example.invalid'},encoding:'utf8'});
    mainGit(['init','--quiet','-b','main']);mainGit(['remote','add','origin','https://github.com/owned/fixture.git']);fs.writeFileSync(path.join(main,'notes'),'original notes\n');mainGit(['add','notes']);mainGit(['commit','--quiet','-m','baseline']);mainGit(['worktree','add','--quiet','-b','linked',project]);
    const metadataDir=path.join(main,'.git/worktrees/project'),mainIndex=fs.readFileSync(path.join(main,'.git/index')),mainHead=mainGit(['rev-parse','HEAD']).trim();
    await js(`document.dispatchEvent(new KeyboardEvent('keydown',{key:'o',ctrlKey:true,bubbles:true}));true`);
    await wait(()=>js(`!!document.querySelector('input[aria-label="Project folder"]')`),'project dialog');await fill('input[aria-label="Project folder"]',project);await click('Open project');
    await wait(()=>js(`!document.querySelector('input[aria-label="Project folder"]')`),'owned project ready');
    const git=(args,input,extra={})=>execFileSync('/usr/bin/git',['-c','core.hooksPath=/dev/null',...args],{cwd:project,env:{PATH:'/usr/bin:/bin',GIT_CONFIG_NOSYSTEM:'1',GIT_CONFIG_GLOBAL:'/dev/null',GIT_AUTHOR_NAME:'Owned',GIT_COMMITTER_NAME:'Owned',GIT_AUTHOR_EMAIL:'owned@example.invalid',GIT_COMMITTER_EMAIL:'owned@example.invalid',...extra},input,encoding:'utf8'});
    const baseline=git(['rev-parse','HEAD']).trim();
    fs.writeFileSync(path.join(project,'notes'),'user staged bytes\r\n');git(['add','notes']);const staged=git(['rev-parse',':notes']).trim();fs.writeFileSync(path.join(project,'notes'),'later user dirty bytes\r\n');const index=fs.readFileSync(path.join(metadataDir,'index'));
    const extra={GIT_INDEX_FILE:path.join(work,'owned-target-index')};git(['read-tree',baseline],undefined,extra);const blob=git(['hash-object','-w','--no-filters','--stdin'],'owned recovered file\n').trim();git(['update-index','--add','--cacheinfo',`100644,${blob},recovered.txt`],undefined,extra);const tree=git(['write-tree'],undefined,extra).trim();targetSha=git(['commit-tree',tree,'-p',baseline,'-m','owned recovery target']).trim();git(['update-ref','refs/heads/ui-recovery-target',targetSha]);
    const native=async()=>(await probe({native_view:true})).view;
    const openLocal=async()=>{await click('GitHub');await click('owned/fixture');await js(`(()=>{const e=[...document.querySelectorAll('summary')].find(e=>e.textContent==='Local Git — staging, commits, branches, fetch and push');if(!e)return false;if(!e.parentElement.open)e.click();return true})()`);await wait(()=>js(`!!document.querySelector('select[aria-label="Local Git operation"]')`),'linked controls');};
    await openLocal();await select('select[aria-label="Local Git operation"]','stage');await fill('textarea[aria-label="Git paths"]','notes');await click('Review local Git request');await wait(()=>js(`!!document.querySelector('[aria-label="Local Git"] [role="alert"]')`),'missing metadata grant refuses');assert.deepEqual(fs.readFileSync(path.join(metadataDir,'index')),index);
    await click('Back to chat');await click('Chat folders');await fill('#chat-folder-path',main);await click('Add folder');await wait(async()=>(await native()).details.context.folder_access.folders.length===2,'read-only common grant');await click('Close Chat folders');
    await openLocal();await select('select[aria-label="Local Git operation"]','stage');await fill('textarea[aria-label="Git paths"]','notes');await click('Review local Git request');await wait(()=>js(`document.querySelector('[aria-label="Local Git"] [role="alert"]')?.textContent.includes('read-only')`),'read-only common metadata refuses writes');assert.deepEqual(fs.readFileSync(path.join(metadataDir,'index')),index);
    await click('Back to chat');await click('Chat folders');await select(`select[aria-label=${JSON.stringify('Access for '+main)}]`,'read_write');await wait(async()=>(await native()).details.context.folder_access.folders[1].mode==='read_write','explicit writable common grant');await click('Close Chat folders');
    await click('GitHub');await click('owned/fixture');
    await js(`(()=>{const e=[...document.querySelectorAll('summary')].find(e=>e.textContent==='Local Git — staging, commits, branches, fetch and push');if(!e)return false;e.click();return true})()`);
    await wait(()=>js(`!!document.querySelector('select[aria-label="Local Git operation"]')`),'local Git controls');
    await select('select[aria-label="Local Git operation"]','stage');await fill('textarea[aria-label="Git paths"]','notes');await click('Review local Git request');await wait(()=>js(`!!document.querySelector('[aria-label="Exact local Git preview"]')`),'storage budget preview');
    const budgetPreview=await js(`JSON.parse(document.querySelector('[aria-label="Exact local Git preview"]').textContent)`);assert.equal(budgetPreview.storage.repository_limit,4294967296);assert.equal(budgetPreview.storage.journal_limit,1073741824);
    const growth=path.join(main,'.git/objects/owned-quota-growth'),growthFd=fs.openSync(growth,'wx',0o600);fs.ftruncateSync(growthFd,4294967296+1);fs.closeSync(growthFd);
    await click('Confirm local Git request');await wait(()=>js(`document.querySelector('[aria-label="Local Git"] [role="alert"]')?.textContent.includes('Git limit reached')`),'late aggregate growth blocks confirmation');assert.deepEqual(fs.readFileSync(path.join(metadataDir,'index')),index);assert.ok(fs.existsSync(growth));
    await probe({size:[900,700]});await wait(()=>js('innerWidth===900'),'quota narrow error');await js(`document.querySelector('[aria-label="Local Git"] [role="alert"]').scrollIntoView({block:'center'});true`);assert.ok(await js('document.documentElement.scrollWidth<=901'));await probe({screenshot:'github-quota-refusal-900.png'});
    await select('select[aria-label="Local Git operation"]','status');await click('Review local Git request');await click('Confirm local Git request');await wait(()=>js(`!!document.querySelector('[aria-label="Local Git result"]')`),'status remains available at quota');assert.ok(fs.existsSync(growth));fs.unlinkSync(growth);
    await select('select[aria-label="Local Git operation"]','checkout');await fill('input[aria-label="Checkout branch"]','ui-recovery-target');await fill('input[aria-label="Checkout target SHA"]',targetSha);
    await click('Review local Git request');await wait(()=>js(`!!document.querySelector('[aria-label="Exact local Git preview"]')`),'UI checkout review');
    const preview=await js(`JSON.parse(document.querySelector('[aria-label="Exact local Git preview"]').textContent)`);
    assert.ok(preview.recovery_path.startsWith(project+'/.ag1-git-recovery-'));assert.equal(preview.metadata.linked,true);assert.equal(preview.metadata.common.path,path.join(main,'.git'));assert.equal(preview.metadata.git.path,metadataDir);
    fs.mkdirSync(preview.recovery_path);fs.writeFileSync(path.join(preview.recovery_path,'later-user'),'preserve later collision');
    await click('Confirm local Git request');await wait(()=>js(`document.querySelector('[aria-label="Local Git"] [role="alert"]')?.textContent.includes('uncertain')`),'truthful interrupted checkout');
    assert.equal(git(['rev-parse','HEAD']).trim(),baseline);assert.deepEqual(fs.readFileSync(path.join(metadataDir,'index')),index);assert.equal(fs.existsSync(path.join(project,'recovered.txt')),false);
    await probe({size:[900,700]});await wait(()=>js('innerWidth===900'),'narrow recovery');
    await click('Inspect recovery');await click('Review local Git request');await click('Confirm local Git request');await wait(()=>js(`!!document.querySelector('select[aria-label="Git recovery choice"]')`),'native recovery choices');
    assert.equal(await js(`([...document.querySelectorAll('button')].find(b=>b.textContent==='Review local Git request')).disabled`),true);
    await select('select[aria-label="Git recovery choice"]','restore');await click('Review local Git request');
    await wait(()=>js(`document.querySelector('[aria-label="Reviewed Git change"]')?.textContent.includes('restore recorded starting state')`),'native restore preview');
    await js(`([...document.querySelectorAll('button')].find(b=>b.textContent==='Confirm local Git request')).scrollIntoView({block:'center'});true`);await probe({screenshot:'github-recovery-restore-preview-900.png'});
    await click('Confirm local Git request');await wait(()=>js(`document.body.innerText.includes('Recorded starting state restored.')`),'native restore complete');await probe({screenshot:'github-recovery-restore-result-900.png'});
    assert.equal(git(['rev-parse','HEAD']).trim(),baseline);assert.deepEqual(fs.readFileSync(path.join(metadataDir,'index')),index);assert.equal(fs.readFileSync(path.join(preview.recovery_path,'later-user'),'utf8'),'preserve later collision');
    await click('Back to chat');await fill('textarea[aria-label="Message AG1"]','Check out model-recovery-target at the exact owned fixture commit. Preserve all unrelated user files.');await click('Send message');
    await wait(()=>js(`!!document.querySelector('.approval-review')`),'model checkout approval');
    const modelPreview=await js(`JSON.parse(document.querySelector('.approval-review [aria-label="Exact local Git preview"]').textContent)`);assert.ok(modelPreview.recovery_path.startsWith(project+'/.ag1-git-recovery-'));
    recoveryId=path.basename(modelPreview.recovery_path);fs.mkdirSync(modelPreview.recovery_path);fs.writeFileSync(path.join(modelPreview.recovery_path,'later-user'),'preserve model collision');
    await click('Allow this operation');await wait(()=>generations===2,'model receives uncertainty');await wait(()=>js(`document.body.innerText.includes('Owned checkout interrupted; no automatic retry.')`),'model interruption message');
    assert.equal(fs.existsSync(path.join(project,'recovered.txt')),false);
    await fill('textarea[aria-label="Message AG1"]','Finish the recorded native recovery using a fresh confirmation. Do not repeat the old checkout or overwrite later work.');await click('Send message');
    await wait(()=>js(`!!document.querySelector('.approval-review')`),'new model recovery approval');assert.ok(await js(`document.querySelector('.approval-review').textContent.includes('recover')`));assert.equal(git(['rev-parse','HEAD']).trim(),baseline);
    await probe({screenshot:'github-recovery-model-approval-900.png'});await click('Allow this operation');await wait(()=>generations===4&&git(['rev-parse','HEAD']).trim()===targetSha,'native model recovery publication');
    await wait(()=>js(`document.body.innerText.includes('Owned native recovery finished; retained originals remain.')`),'model recovery result');await probe({screenshot:'github-recovery-model-result-900.png'});
    assert.equal(fs.readFileSync(path.join(project,'recovered.txt'),'utf8'),'owned recovered file\n');assert.equal(git(['rev-parse',':notes']).trim(),staged);assert.equal(fs.readFileSync(path.join(project,'notes'),'utf8'),'later user dirty bytes\r\n');assert.equal(fs.readFileSync(path.join(modelPreview.recovery_path,'later-user'),'utf8'),'preserve model collision');
    const bucket=fs.readdirSync(path.join(work,'state')).find(n=>n.startsWith('transition-'));assert.ok(bucket);const directory=path.join(work,'state',bucket);const active=JSON.parse(fs.readFileSync(path.join(directory,'active.json'),'utf8'));const record=JSON.parse(fs.readFileSync(path.join(directory,active.id,'record.json'),'utf8'));assert.equal(record.recovery_of.id,recoveryId);assert.ok(fs.existsSync(path.join(directory,active.id,'completed.json')));
    // Exercise the remaining staging types through the same actual native UI.
    fs.renameSync(path.join(project,'recovered.txt'),path.join(work,'retained-user-deletion'));
    fs.symlinkSync('../owned-unfollowed-target',path.join(project,'owned-staged-link'));
    const beforeStage=fs.readFileSync(path.join(metadataDir,'index'));
    await click('GitHub');await click('owned/fixture');
    await js(`(()=>{const e=[...document.querySelectorAll('summary')].find(e=>e.textContent==='Local Git — staging, commits, branches, fetch and push');if(!e)return false;if(!e.parentElement.open)e.click();return true})()`);
    await wait(()=>js(`!!document.querySelector('select[aria-label="Local Git operation"]')`),'staging controls');
    await select('select[aria-label="Local Git operation"]','stage');await fill('textarea[aria-label="Git paths"]','recovered.txt\nowned-staged-link');await click('Review local Git request');
    await wait(()=>js(`document.querySelector('[aria-label="Reviewed Git change"]')?.textContent.includes('stage deletion (index only)')`),'deletion and link preview');
    assert.ok(await js(`document.querySelector('[aria-label="Reviewed Git change"]').textContent.includes('stage link target bytes (not followed)')`));assert.deepEqual(fs.readFileSync(path.join(metadataDir,'index')),beforeStage);
    await js(`document.querySelector('[aria-label="Reviewed Git change"]').scrollIntoView({block:'start'});true`);await probe({screenshot:'github-stage-link-deletion-900.png'});
    await click('Confirm local Git request');await wait(()=>js(`!!document.querySelector('[aria-label="Local Git result"]')`),'staging finished');
    assert.equal(git(['ls-files','--','recovered.txt']).trim(),'');assert.ok(git(['ls-files','--stage','--','owned-staged-link']).startsWith('120000 '));assert.equal(git(['show',':owned-staged-link']),'../owned-unfollowed-target');
    assert.equal(fs.readFileSync(path.join(work,'retained-user-deletion'),'utf8'),'owned recovered file\n');assert.equal(fs.readlinkSync(path.join(project,'owned-staged-link')),'../owned-unfollowed-target');assert.equal(git(['rev-parse',':notes']).trim(),staged);
    const calls=fs.readFileSync(path.join(work,'state/github-requests.jsonl'),'utf8').trim().split('\n').filter(Boolean).map(JSON.parse);assert.ok(calls.every(r=>r.operation.risk==='read'));assert.deepEqual(errors,[]);assert.equal(requests.length,4);
    assert.deepEqual(fs.readFileSync(path.join(main,'.git/index')),mainIndex);assert.equal(mainGit(['rev-parse','HEAD']).trim(),mainHead);assert.equal(fs.readFileSync(path.join(main,'notes'),'utf8'),'original notes\n');
    return {promptsSubmitted:2,providerRequests:requests,githubCalls:calls,liveGitHub:false,checks:['native budget preview and late aggregate growth recheck','quota refusal preserves index and retained bytes; status remains available','missing and read-only metadata grants block all writes','explicit writable metadata shown in review','main worktree index/HEAD/files preserved','native checkout uncertainty created without forged private records','UI inspection and explicit restore choice require new confirmation','later recovery-folder collision content preserved','model uncertain result never automatically retried','model recovery uses separate native approval and exact pending identity','actual reviewed file publication preserves unrelated staged/dirty bytes','private chained recovery records completed with earlier originals retained','readable model Git preview shares the UI summary with collapsed exact data','actual UI stages deleted entry and link target bytes without following targets or altering working files']};
  }
});
