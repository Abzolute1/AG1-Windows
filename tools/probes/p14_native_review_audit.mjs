// Private native WebKit review-panel audit. No inference or external writes.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { runNativeFixture } from './p14_native_startup_smoke.mjs';

await runNativeFixture(process.argv[2], { prefix: 'review-', gitReview: true,
  async exercise({ work, probe, wait, requests }) {
    const project = path.join(work, 'project'); fs.mkdirSync(project, { mode: 0o700 });
    const downloads = path.join(work, 'downloads'); fs.mkdirSync(downloads, { mode: 0o700 });
    fs.writeFileSync(path.join(work, 'xdg-config', 'user-dirs.dirs'), `XDG_DOWNLOAD_DIR="${downloads}"\n`);
    fs.writeFileSync(path.join(project, 'notes.txt'), 'Unchanged project source\n');
    const git = args => execFileSync('/usr/bin/git', ['-c', 'core.hooksPath=/dev/null', ...args], {
      cwd: project, env: { PATH: '/usr/bin:/bin', GIT_CONFIG_NOSYSTEM: '1', GIT_CONFIG_GLOBAL: '/dev/null',
        GIT_AUTHOR_NAME: 'Owned fixture', GIT_AUTHOR_EMAIL: 'fixture@example.invalid',
        GIT_COMMITTER_NAME: 'Owned fixture', GIT_COMMITTER_EMAIL: 'fixture@example.invalid' } });
    git(['init', '--quiet']); git(['add', '--', 'notes.txt']); git(['commit', '--quiet', '-m', 'Owned review baseline']);
    fs.writeFileSync(path.join(project, 'notes.txt'), 'User dirty source retained for workspace review\n');
    const js = script => probe({ script });
    const click = label => wait(() => js(`(()=>{const b=[...document.querySelectorAll('button')].find(b=>b.getClientRects().length&&(b.getAttribute('aria-label')===${JSON.stringify(label)}||b.textContent.trim()===${JSON.stringify(label)}));if(!b||b.disabled)return false;b.focus();b.click();return true})()`), `click ${label}`);
    const fill = (selector, value, type = 'HTMLInputElement') => js(`(()=>{const e=document.querySelector(${JSON.stringify(selector)});if(!e)return false;Object.getOwnPropertyDescriptor(${type}.prototype,'value').set.call(e,${JSON.stringify(value)});e.dispatchEvent(new Event('input',{bubbles:true}));return true})()`);
    await click('Back to chat');
    await js(`document.dispatchEvent(new KeyboardEvent('keydown',{key:'o',ctrlKey:true,bubbles:true}));true`);
    await wait(() => js(`!!document.querySelector('input[aria-label="Project folder"]')`), 'project dialog');
    await fill('input[aria-label="Project folder"]', project); await click('Open project');
    await wait(() => js(`!document.querySelector('input[aria-label="Project folder"]')`), 'project opened');
    assert.equal((await probe({ native_view: true })).view.snapshot.workspace_path, project);
    const source = '# Retained artifact\n\nReadable without nested activity.\n<script>This is literal source, never executable.</script>\n';
    const digest = crypto.createHash('sha256').update(source).digest('hex');
    // Seed native artifact custody through the existing authenticated upload IPC,
    // not renderer mocks. No message submission or model generation follows.
    await js(`(()=>{window.reviewSeed={};(async()=>{const invoke=window.__TAURI_INTERNALS__.invoke;const call=async command=>{const {snapshot:s}=await invoke('bridge_view');const r=await invoke('bridge_command',{envelope:{version:s.version,id:crypto.randomUUID(),scope:s.scope,expected_revision:s.revision,command}});if(['failed','rejected'].includes(r.status))throw Error(JSON.stringify(r));return r.data};const {upload}=await call({kind:'artifacts',action:'upload_begin',args:{}});await call({kind:'artifacts',action:'upload_append',args:{upload,offset:'0',base64:${JSON.stringify(Buffer.from(source).toString('base64'))}}});window.reviewSeed.result=await call({kind:'artifacts',action:'upload_finish',args:{upload,sha256:${JSON.stringify(digest)}}});})().catch(e=>window.reviewSeed.error=String(e));return true})()`);
    await wait(() => js(`!!(window.reviewSeed.result||window.reviewSeed.error)`), 'native artifact seed');
    assert.equal(await js(`window.reviewSeed.error||null`), null);
    const draft = 'Keep this draft while reviewing artifacts.';
    await fill('textarea[aria-label="Message AG1"]', draft, 'HTMLTextAreaElement');
    await click('Artifacts');
    await wait(() => js(`!!document.querySelector('.artifact-list button')`), 'artifact list');
    await js(`document.querySelector('.artifact-list button').click();true`);
    await wait(() => js(`document.querySelector('.artifact-source')?.textContent.includes('Readable without nested activity')`), 'native artifact bytes');
    assert.equal(await js(`document.querySelector('.artifact-source script')!==null`), false);
    await click('Prepare download');
    await wait(() => js(`!!document.querySelector('.artifact-actions a[download]')`), 'verified download');
    await js(`document.querySelector('.artifact-actions a[download]').click();true`);
    const download = await js(`document.querySelector('.artifact-actions a[download]').download`);
    await wait(() => fs.existsSync(path.join(downloads, download)), 'original download written');
    assert.equal(fs.readFileSync(path.join(downloads, download), 'utf8'), source);
    const measurements = [];
    // Respect the product window's declared 900x600 minimum.
    for (const [width, height] of [[1440,900],[900,600]]) {
      await probe({ size: [width,height] });
      await wait(() => js(`innerWidth===${width}&&innerHeight===${height}`), 'native resize');
      const layout = await js(`(()=>{const p=document.querySelector('.review-panel'),r=p.getBoundingClientRect();return {width:innerWidth,pageWidth:document.documentElement.scrollWidth,x:r.x,right:r.right,height:r.height,mainInert:document.querySelector('.main').inert,role:p.getAttribute('role'),draft:document.querySelector('textarea[aria-label="Message AG1"]').value}})()`);
      assert.ok(layout.x >= 0 && layout.right <= width + 1 && layout.pageWidth <= width + 1, JSON.stringify(layout));
      assert.equal(layout.draft, draft); assert.equal(layout.mainInert, width <= 1100);
      measurements.push(layout); await probe({ screenshot: `artifacts-${width}.png` });
    }
    await click('Close review panel');
    await wait(() => js(`document.activeElement?.getAttribute('aria-label')==='Artifacts'`), 'opener focus restored');
    assert.equal(await js(`document.querySelector('.main').inert`), false);
    await click('Artifacts');
    assert.equal(await js(`document.querySelector('.artifact-source').textContent`), source);
    await click('Review');
    await wait(() => js(`!!document.querySelector('.review-file-list button')`), 'actual native Git diff');
    assert.equal(await js(`document.querySelector('.workspace-review [role="alert"]')?.textContent||null`), null);
    await js(`document.querySelector('.review-file-list button').click();true`);
    await wait(() => js(`document.querySelector('.review-diff')?.textContent.includes('+User dirty source retained for workspace review')`), 'native Git postimage');
    assert.ok(await js(`document.querySelector('.review-diff').textContent.includes('-Unchanged project source')`));
    await probe({ screenshot: 'review-git-diff-900.png' });
    await click('Artifacts'); await click('Expand review panel');
    await probe({ size: [1440,900] }); await wait(() => js(`innerWidth===1440`), 'wide expanded panel');
    assert.equal(await js(`document.querySelector('.review-panel').getBoundingClientRect().width`), 1440);
    await click('Restore panel size');
    await wait(() => js(`!document.querySelector('.main').inert`), 'nonmodal side panel restored');
    await js(`document.querySelector('[aria-label="Resize review panel"]').dispatchEvent(new KeyboardEvent('keydown',{key:'Home',bubbles:true}));true`);
    await wait(() => js(`document.querySelector('[aria-label="Resize review panel"]').getAttribute('aria-valuenow')==='320'`), 'keyboard width');
    await probe({ screenshot: 'artifacts-min-width.png' });
    await click('Close review panel');
    assert.equal(await js(`document.querySelector('textarea[aria-label="Message AG1"]').value`), draft);
    assert.equal(fs.readFileSync(path.join(project,'notes.txt'),'utf8'), 'User dirty source retained for workspace review\n');
    assert.deepEqual(requests, []);
    fs.writeFileSync(path.join(work, 'review-layout.json'), JSON.stringify(measurements,null,2));
    return { promptsSubmitted: 0, providerRequests: [], checks: ['native artifact custody and literal source', 'digest-checked original download', 'wide and narrow review layout', 'draft and selection preserved', 'keyboard resize and opener focus restoration', 'overlay inert cleanup', 'actual Git diff through configured native helper; user dirty bytes preserved'] };
  }
});
