// Native WebKit/Monaco + file IPC; no prompt or provider request is permitted.
// All edits and simulated external changes are confined to disposable files.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import vm from 'node:vm';
import { runNativeFixture } from './p14_native_startup_smoke.mjs';

await runNativeFixture(process.argv[2], { prefix: 'editor-',
  async exercise({ work, probe, wait, requests }) {
    const project = path.join(work, 'project');
    fs.mkdirSync(project, { mode: 0o700 });
    const name = `notes-${'long-file-name-'.repeat(9)}.txt`;
    const original = 'Original editor fixture\n';
    const prefix = 'Unsaved local change: ';
    const external = 'External replacement during editing\n';
    fs.writeFileSync(path.join(project, name), original);
    fs.writeFileSync(path.join(project, 'other.txt'), 'Other file stays unchanged\n');
    const js = async script => {
      try { new vm.Script(script); return await probe({ script }); }
      catch (error) { fs.writeFileSync(path.join(work, 'failed-script.txt'), script); throw error; }
    };
    const click = label => wait(() => js(`(()=>{const b=[...document.querySelectorAll('button')].find(b=>b.getAttribute('aria-label')===${JSON.stringify(label)}||b.textContent.trim()===${JSON.stringify(label)});if(!b||b.disabled)return false;b.click();return true})()`), `click ${label}`);
    const fill = async (selector, value) => assert.equal(await js(`(()=>{const e=document.querySelector(${JSON.stringify(selector)});if(!e)return false;Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(e,${JSON.stringify(value)});e.dispatchEvent(new Event('input',{bubbles:true}));return true})()`), true);
    const dirty = () => js(`!!document.querySelector('[aria-label="Unsaved changes"]')`);
    const lines = async () => {
      const value = await js(String.raw`[...document.querySelectorAll('.monaco-editor .view-line')].map(e=>e.textContent.replace(/\u00a0/g,' ')).join('\n')`);
      fs.writeFileSync(path.join(work, 'visible-editor-text.json'), JSON.stringify(value));
      // Monaco renders soft-wrapped screen lines separately. This check is for
      // visible content; native save checks below compare exact disk bytes.
      return value.replace(/\s+/g, ' ');
    };
    const hasFile = file => js(`[...document.querySelectorAll('.file-tree button')].some(b=>b.textContent===${JSON.stringify(file)})`);
    const selected = () => js(`document.querySelector('.file-toolbar > span')?.textContent`);
    const measurements = [], checks = [];
    const measure = () => js(`(()=>{const rect=s=>{const r=document.querySelector(s)?.getBoundingClientRect();return r?{x:r.x,y:r.y,width:r.width,height:r.height,right:r.right,bottom:r.bottom}:null};return {width:innerWidth,height:innerHeight,pageWidth:document.documentElement.scrollWidth,pane:rect('.file-pane'),toolbar:rect('.file-toolbar'),title:rect('.file-toolbar > span'),host:rect('.file-pane .editor'),monaco:rect('.file-pane .monaco-editor'),buttons:[...document.querySelectorAll('.file-toolbar button')].map(b=>{const r=b.getBoundingClientRect();return {label:b.textContent,x:r.x,right:r.right,bottom:r.bottom}})}})()`);
    const layout = async (width, height, suffix = '') => {
      await probe({ size: [width, height] });
      await wait(async () => {
        const m = await measure();
        return m.width === width && m.height === height && m.monaco && m.host &&
          Math.abs(m.monaco.width - m.host.width) < 2 && Math.abs(m.monaco.height - m.host.height) < 2;
      }, 'Monaco follows native resize');
      const m = await measure(); measurements.push(m);
      fs.writeFileSync(path.join(work, 'layout.json'), JSON.stringify(measurements, null, 2));
      assert.ok(m.monaco.width > m.pane.width * .95, JSON.stringify(m));
      assert.ok(m.monaco.height > (m.pane.height - m.toolbar.height) * .95, JSON.stringify(m));
      assert.ok(m.monaco.height > height * .65, JSON.stringify(m));
      assert.ok(m.pageWidth <= width + 1 && m.monaco.right <= width + 1 && m.monaco.bottom <= height + 1, JSON.stringify(m));
      assert.ok(m.title.right <= m.buttons[0].x, JSON.stringify(m));
      for (let i = 1; i < m.buttons.length; i++) assert.ok(m.buttons[i - 1].right < m.buttons[i].x, JSON.stringify(m));
      assert.ok(m.buttons.at(-1).right <= width, JSON.stringify(m));
      await probe({ screenshot: `editor-${width}${suffix}.png` });
    };

    await click('Back to chat');
    await js(`document.dispatchEvent(new KeyboardEvent('keydown',{key:'o',ctrlKey:true,bubbles:true}));true`);
    await wait(() => js(`!!document.querySelector('input[aria-label="Project folder"]')`), 'project dialog');
    await fill('input[aria-label="Project folder"]', project);
    await click('Open project');
    await wait(() => js(`!document.querySelector('input[aria-label="Project folder"]')`), 'project opened');
    assert.equal((await probe({ native_view: true })).view.snapshot.workspace_path, project);
    await click('Files');
    await wait(() => hasFile(name), 'initial file list');
    await click(name);
    await wait(() => js(`!!document.querySelector('.monaco-editor textarea')`), 'native Monaco loaded');
    await wait(async () => (await lines()).includes(original.trim()), 'initial content');
    await layout(1440, 900);
    await layout(900, 600);
    await click('Hide sidebar');
    await layout(900, 600, '-sidebar-hidden');
    await click('Show sidebar');
    checks.push('Monaco fills pane after resize/sidebar toggle; long filename does not overlap toolbar actions');

    // Use the editor's actual DOM input path, not a replacement React textarea
    // or direct mutation of component/model internals.
    const insert = async text => {
      assert.equal(await js(`(()=>{const e=document.querySelector('.monaco-editor textarea');e.focus();return document.execCommand('insertText',false,${JSON.stringify(text)})})()`), true);
      await wait(dirty, 'local edit becomes dirty');
    };
    await insert(prefix);
    await wait(async () => (await lines()).includes(prefix + original.trim()), 'local editor content');
    const marker = await js(`(()=>{const e=document.querySelector('[aria-label="Unsaved changes"]'),p=document.querySelector('.file-toolbar > span');const r=e.getBoundingClientRect(),b=p.getBoundingClientRect();return {left:r.left,right:r.right,parentLeft:b.left,parentRight:b.right}})()`);
    fs.writeFileSync(path.join(work, 'dirty-marker.json'), JSON.stringify(marker, null, 2));
    assert.ok(marker.left >= marker.parentLeft && marker.right <= marker.parentRight, 'Long filename must not clip the dirty marker: ' + JSON.stringify(marker));
    assert.equal(fs.readFileSync(path.join(project, name), 'utf8'), original);
    const unchangedBuffer = async () => {
      assert.equal(await dirty(), true);
      assert.ok((await lines()).includes(prefix + original.trim()));
      assert.ok((await selected()).startsWith(name));
    };

    fs.writeFileSync(path.join(project, 'created.txt'), 'Created while Files is open\n');
    const began = Date.now();
    await wait(() => hasFile('created.txt'), 'automatic creation refresh');
    const automaticRefreshMs = Date.now() - began;
    await unchangedBuffer();
    fs.renameSync(path.join(project, 'created.txt'), path.join(project, 'renamed.txt'));
    await wait(async () => await hasFile('renamed.txt') && !await hasFile('created.txt'), 'automatic rename refresh');
    await unchangedBuffer();
    // Move outside the project to test disappearance while preserving bytes.
    fs.renameSync(path.join(project, 'renamed.txt'), path.join(work, 'removed-from-project.txt'));
    await wait(async () => !await hasFile('renamed.txt'), 'automatic removal refresh');
    await unchangedBuffer();
    fs.writeFileSync(path.join(project, 'manual.txt'), 'Manual refresh fixture\n');
    await click('Refresh files');
    await wait(() => hasFile('manual.txt'), 'manual refresh');
    await unchangedBuffer();
    checks.push('automatic create/rename/remove and explicit refresh preserve selected dirty buffer');

    await click('other.txt');
    await wait(() => js(`document.body.innerText.includes('Save or discard your changes before opening another file')`), 'dirty navigation refusal');
    await unchangedBuffer();
    fs.writeFileSync(path.join(project, name), external);
    await click('Save');
    await wait(() => js(`/conflict/i.test(document.body.innerText)`), 'native stale revision rejected');
    const conflictMessage = await js(`document.querySelector('.error[role="alert"]')?.textContent`);
    assert.ok(conflictMessage?.includes('Save conflict: the file changed on disk.'));
    assert.ok(conflictMessage.includes('Your unsaved text is still in the editor.'));
    assert.ok(conflictMessage.includes('Copy it somewhere safe before choosing Discard.'));
    assert.ok(conflictMessage.includes('Reopen the file from the file list'));
    assert.ok(!conflictMessage.includes('{"error":'));
    fs.writeFileSync(path.join(work, 'conflict-message.json'), JSON.stringify(conflictMessage));
    assert.equal(fs.readFileSync(path.join(project, name), 'utf8'), external);
    await unchangedBuffer();
    await probe({ screenshot: 'editor-conflict-dirty.png' });
    checks.push('dirty file switch refused; conflicting save explains safe recovery and preserves local buffer and external disk bytes');

    await click('Discard');
    await wait(async () => !await dirty(), 'discard resets buffer');
    await click(name);
    await wait(async () => (await lines()).includes(external.trim()), 'explicit reopen reads current external bytes');
    await insert('Saved change: ');
    await click('Save');
    await wait(async () => !await dirty(), 'successful save clears dirty marker');
    assert.equal(fs.readFileSync(path.join(project, name), 'utf8'), 'Saved change: ' + external);
    assert.equal(fs.readFileSync(path.join(project, 'other.txt'), 'utf8'), 'Other file stays unchanged\n');
    checks.push('discard/reopen recovery and exact successful save');
    assert.deepEqual(requests, []);
    await probe({ screenshot: 'editor-final.png' });
    return { promptsSubmitted: 0, provider: 'rejecting loopback sink; zero requests', checks,
      measurements, automaticRefreshMs, project,
      limits: ['diagnostic DOM input, not physical keyboard', 'no model-driven file creation', 'no cross-project delayed-response race test'] };
  },
});
