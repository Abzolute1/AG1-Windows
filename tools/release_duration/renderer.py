"""Actual WebKit reload and reattachment during a disposable native repair."""
import base64
import hashlib
import json
import os
from pathlib import Path
import struct
import subprocess
import sys
import time
import uuid
from types import SimpleNamespace
from faults import retain
from resources import descendants, identity, same_process

ROOT = Path.cwd()
sys.path.insert(0, str(ROOT / 'tools/probes'))
import desktop_visual as ui


def visible_excerpt(markdown):
    # Use the renderer's Markdown parser. Literal source backticks/emphasis do
    # not occur in DOM innerText, and regex stripping corrupts links/code.
    script = '''
import {unified} from 'unified';
import parse from 'remark-parse';
import {toString} from 'mdast-util-to-string';
import fs from 'node:fs';
const tree = unified().use(parse).parse(fs.readFileSync(0, 'utf8'));
const candidates = [];
function visit(node) {
  if (['paragraph', 'heading', 'code'].includes(node.type)) {
    const text = toString(node).replace(/\\s+/g, ' ').trim();
    if (text.length >= 20 && !JSON.stringify(node).includes('"type":"image"')) candidates.push(text);
  } else for (const child of node.children ?? []) visit(child);
}
visit(tree);
if (!candidates.length) throw Error('No meaningful rendered text excerpt');
process.stdout.write(candidates[0].slice(0, 100));
'''
    result = subprocess.run(['node', '--input-type=module', '-e', script],
                            input=markdown, text=True, capture_output=True, check=True,
                            cwd=ROOT / 'product/apps/desktop', timeout=30)
    return result.stdout


def run(binary, workspace, config, output, task, fixture, sampler=None, heap_library=None, after_task=None):
    output = Path(output)
    output.mkdir(mode=0o700)
    display = next(n for n in range(100, 500) if not Path(f'/tmp/.X{n}-lock').exists()
                   and not Path(f'/tmp/.X11-unix/X{n}').exists())
    auth = output / 'display.auth'
    auth.write_bytes(struct.pack('>H', 65535) + b''.join(struct.pack('>H', len(v)) + v for v in
        [b'', str(display).encode(), b'MIT-MAGIC-COOKIE-1', os.urandom(16)]))
    auth.chmod(0o600)
    socket = output / 'control.sock'
    if len(os.fsencode(socket)) > 100:
        raise RuntimeError('Private renderer control path is too long')
    env = dict(os.environ, DISPLAY=f':{display}', XAUTHORITY=str(auth), GDK_BACKEND='x11',
               LIBGL_ALWAYS_SOFTWARE='1', WEBKIT_DISABLE_DMABUF_RENDERER='1',
               AG1_DESKTOP_STATE=str(output / 'desktop-state'), AG1_VISUAL_SOCKET=str(socket),
               AG1_NATIVE_CONFIG=str(config))
    for key in ('AG1_FIXTURE_PROVIDER', 'AG1_CUTOVER_WORKSPACE', 'AG1_RELEASE_WORKSPACE'):
        env.pop(key, None)
    env['AG1_FIXTURE_PROVIDER' if fixture else 'AG1_RELEASE_WORKSPACE'] = '1' if fixture else str(workspace)
    if heap_library:
        env.update(LD_PRELOAD=str(heap_library), AG1_HEAP_SAMPLE_PREFIX=str(output / 'heap'),
                   AG1_HEAP_SAMPLE_SECONDS='60')
    xvfb = desktop = None
    owned = {}
    report = {'status': 'running', 'fixture': fixture, 'release_acceptance': False,
              'started_monotonic_ns': time.monotonic_ns(), 'observations': [],
              'scenario': 'actual WebKit page reload while native command runs, then project UI reattachment'}
    def view():
        return ui.request({'native_view': True})['view']
    def invoke(envelope):
        return ui.async_js('return await window.__TAURI_INTERNALS__.invoke("bridge_command",{envelope:'
                           + json.dumps(envelope) + '});')
    def open_project():
        ui.wait('!!document.querySelector(".workspace-pill")', 30)
        ui.js('document.dispatchEvent(new KeyboardEvent("keydown",{key:"o",ctrlKey:true,bubbles:true}));true')
        ui.wait('!!document.querySelector(".dialog input")')
        ui.text('input[aria-label="Project folder"]', str(workspace))
        ui.js('document.querySelector(".dialog form").requestSubmit();true')
        ui.wait('!!document.querySelector(\'textarea[aria-label="Message AG1"]\')', 1800)
    try:
        with (output / 'xvfb.log').open('x') as log:
            xvfb = subprocess.Popen([str(ROOT / '.local/p11/xvfb/usr/bin/Xvfb'), f':{display}',
                '-screen', '0', '1440x900x24', '-nolisten', 'tcp', '-auth', str(auth)],
                stdout=log, stderr=subprocess.STDOUT)
        deadline = time.monotonic() + 10
        while not Path(f'/tmp/.X11-unix/X{display}').exists():
            if xvfb.poll() is not None or time.monotonic() > deadline:
                raise RuntimeError('Private display failed to start')
            time.sleep(.05)
        with (output / 'desktop.log').open('x') as log:
            desktop = subprocess.Popen([str(binary)], cwd=ROOT, env=env, stdout=log,
                                        stderr=subprocess.STDOUT)
        deadline = time.monotonic() + 60
        while not socket.exists():
            if desktop.poll() is not None or time.monotonic() > deadline:
                raise RuntimeError('Disposable renderer broker did not start')
            time.sleep(.05)
        ui.SOCKET = socket
        if sampler:
            sampler.bind(SimpleNamespace(process=desktop, output=output))
        open_project()
        initial = view()
        if initial['snapshot']['status'] != 'idle':
            raise RuntimeError('Renderer must begin from settled native state')
        snapshot = initial['snapshot']
        envelope = {'version': snapshot['version'], 'id': 'p14-renderer-' + uuid.uuid4().hex,
                    'scope': snapshot['scope'], 'expected_revision': snapshot['revision'],
                    'command': {'kind': 'prompt', 'input': {'kind': 'inline', 'text': task}}}
        acknowledgement = invoke(envelope)
        if acknowledgement['status'] != 'accepted' or acknowledgement['data']['kind'] != 'accepted':
            raise RuntimeError(f'Renderer prompt was not admitted: {acknowledgement}')
        report.update(before=initial, command=envelope, acknowledgement=acknowledgement)
        deadline = time.monotonic() + 1200
        while True:
            active = view()
            if active['snapshot']['active_operation'] == acknowledgement['data']['operation']:
                break
            if time.monotonic() > deadline or active['error']:
                raise RuntimeError('Renderer command did not become active')
            time.sleep(.05)
        report['before_detach'] = active
        report['detach_monotonic_ns'] = time.monotonic_ns()
        ui.js('setTimeout(()=>location.reload(),0);true')
        ui.wait('document.querySelector(".workspace-pill")?.textContent.trim()==="Attach project"', 30)
        report['detached_home_visible'] = True
        # Keep the fresh page at home while the same native operation settles.
        while True:
            current = view()
            root = identity(desktop.pid)
            if root: owned.update({r['pid']: r for r in descendants(root) if r['pid'] != desktop.pid})
            report['observations'].append({'monotonic_ns': time.monotonic_ns(),
                'scope': current['snapshot']['scope'], 'status': current['snapshot']['status'],
                'revision': current['snapshot']['revision'], 'error': current['error']})
            if current['error'] or current['snapshot']['pending']:
                raise RuntimeError('Detached native work requires recovery or approval')
            if current['snapshot']['status'] == 'idle':
                break
            if time.monotonic() > deadline:
                raise RuntimeError('Detached native command timed out')
            time.sleep(.25)
        query = {**envelope, 'id': 'p14-query-' + uuid.uuid4().hex,
                 'expected_revision': current['snapshot']['revision'],
                 'command': {'kind': 'command_result', 'command': envelope['id']}}
        terminal = invoke(query)
        expected = {'kind': 'settled', 'operation': {
            'operation': acknowledgement['data']['operation'], 'status': 'completed'}}
        if terminal['data'] != expected:
            raise RuntimeError(f'Detached command did not complete: {terminal}')
        report['terminal'] = terminal
        report['before_reattach'] = current
        page_offset = '0'
        context_bytes = bytearray()
        context_sha = None
        while page_offset is not None:
            arguments = {'section': 'context', 'offset': page_offset}
            if context_sha is not None:
                arguments['sha256'] = context_sha
            detail = invoke({**query, 'id': 'p14-context-' + uuid.uuid4().hex,
                'command': {'kind': 'runtime', 'action': 'detail_page', 'args': arguments}})
            if detail['status'] != 'query':
                raise RuntimeError('Full renderer-host context could not be observed')
            body = detail['data']
            context_sha = context_sha or body['sha256']
            if context_sha != body['sha256']:
                raise RuntimeError('Context changed during renderer pagination')
            context_bytes.extend(base64.b64decode(body['base64'], validate=True))
            page_offset = body['next']
        if hashlib.sha256(context_bytes).hexdigest() != context_sha:
            raise RuntimeError('Renderer context hash mismatch')
        report['full_context'] = json.loads(context_bytes)
        report['full_context_sha256'] = context_sha
        report['reattach_started_monotonic_ns'] = time.monotonic_ns()
        open_project()
        ui.wait('document.querySelectorAll(".chat-turn").length>0', 30)
        after = view()
        if after['snapshot']['scope'] != current['snapshot']['scope']:
            raise RuntimeError('Renderer reattachment selected a different session')
        if after['snapshot']['message_count'] != current['snapshot']['message_count']:
            raise RuntimeError('Reattachment changed the acknowledged message count')
        report.update(after_reattach=after, reattached_monotonic_ns=time.monotonic_ns(),
                      dom_text=ui.js('document.body.innerText'))
        assistants = [m for m in after['details']['messages'] if m['role'] == 'assistant']
        texts = [b['text'] for b in assistants[-1]['blocks'] if b['kind'] == 'text' and b['text'].strip()]
        if not texts:
            raise RuntimeError('No completed assistant text is available for renderer verification')
        expected_text = visible_excerpt(texts[-1])
        ui.js('(()=>{const list=document.querySelector(".conversation-scroll");list.scrollTop=list.scrollHeight;return true;})()')
        ui.wait('(()=>{const answers=[...document.querySelectorAll(".final-answer")];'
                'return !!answers.length && answers.at(-1).innerText.replace(/\\s+/g," ").includes('
                +json.dumps(expected_text)+');})()', 30)
        report['rendered_assistant_text_prefix'] = expected_text
        report['dom_text'] = ui.js('document.body.innerText')
        ui.screenshot('reattached')
        if after_task:
            report['navigation_probe'] = after_task(ui, view, desktop)
        root = identity(desktop.pid)
        if root: owned.update({r['pid']: r for r in descendants(root) if r['pid'] != desktop.pid})
        ui.js('setTimeout(()=>window.__TAURI_INTERNALS__.invoke("quit",{}),0);true')
        desktop.wait(timeout=60)
        if desktop.returncode:
            raise RuntimeError(f'Renderer drain failed: {desktop.returncode}')
        deadline = time.monotonic() + 30
        while True:
            survivors = []
            for pid, original in owned.items():
                observed = identity(pid)
                if same_process(original, observed) and observed['state'] != 'Z': survivors.append(observed)
            if not survivors: break
            if time.monotonic() > deadline: raise RuntimeError('Renderer owned children leaked: '+str(survivors))
            time.sleep(.1)
        report.update(status='passed', desktop_exit_code=desktop.returncode, owned_children=list(owned.values()), leaked_owned_children=0)
    except Exception as error:
        report.update(status='failed', error=repr(error))
        raise
    finally:
        if desktop and desktop.poll() is None:
            try:
                ui.js('setTimeout(()=>window.__TAURI_INTERNALS__.invoke("quit",{}),0);true')
                desktop.wait(timeout=60)
            except Exception:
                # Only this explicitly launched desktop is terminated. Its native
                # process supervisors retain descendant cleanup responsibility.
                desktop.terminate()
                desktop.wait(timeout=60)
        if sampler:
            sampler.unbind()
        if xvfb and xvfb.poll() is None:
            xvfb.terminate()
            xvfb.wait(timeout=10)
        report['finished_monotonic_ns'] = time.monotonic_ns()
        retain(output / 'renderer-result.json', report)
    return report
