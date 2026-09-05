"""Real WebKit renderer reload and sandboxed static-preview checks, fixture workspace only."""
import json,time
from desktop_visual import *
project=ROOT/'.local/p04-visual/project'
wait('document.querySelector("footer").innerText.includes("All caught up")')
click('Chat');text('textarea[aria-label="Message AG1"]','[approval] Write the approved fixture');click('Send message')
wait('document.querySelector(".approval-dialog")!==null')
before=async_js('return (await window.__TAURI_INTERNALS__.invoke("bridge_view")).snapshot')
js('window.__beforeReload=true;location.reload();true')
wait('window.__beforeReload===undefined && document.readyState==="complete"')
wait('document.querySelector(".approval-dialog")!==null')
after=async_js('return (await window.__TAURI_INTERNALS__.invoke("bridge_view")).snapshot')
assert before['pending'][0]['binding']==after['pending'][0]['binding']
assert before['scope']==after['scope']
screenshot('ag1-approval-light');js('document.documentElement.classList.add("dark");true');screenshot('ag1-approval-dark');js('document.documentElement.classList.remove("dark");true')
click('Allow this operation');wait('document.querySelector(".approval-dialog")===null')
wait('document.querySelector("footer").innerText.includes("All caught up")')
assert (project/'approved.txt').read_text()=='Approved fixture mutation\n'
payload='''<h1>Isolated preview fixture</h1><p>Static content renders inside this file pane.</p><script>parent.postMessage("script-executed", "*");parent.__TAURI_INTERNALS__.invoke("quit")</script><button onclick="parent.postMessage('handler-executed','*')">Static button</button><iframe src="https://example.invalid/"></iframe><a href="https://example.invalid/" target="_top">Navigation removed</a><form action="https://example.invalid/"><input name="secret"><button>Submit</button></form>'''
(project/'preview.html').write_text(payload)
js('window.__previewMessages=[];window.addEventListener("message",e=>{if(e.source!==window)window.__previewMessages.push(e.data)});true')
click('Files');wait('[...document.querySelectorAll(".file-tree button")].some(b=>b.textContent==="preview.html")');click('preview.html')
wait('document.querySelector(".monaco-editor")!==null');click('Static preview')
wait('!!document.querySelector("iframe[title]")');time.sleep(.5)
probe=js('(()=>{const f=document.querySelector("iframe");f.focus();const r=f.getBoundingClientRect();let denied=false;try{void f.contentWindow.document.body;}catch{denied=true;}return {sandbox:f.getAttribute("sandbox"),referrer:f.referrerPolicy,opaqueOrigin:denied,noParentDocument:f.contentDocument===null,messages:window.__previewMessages,focused:document.activeElement===f,contained:r.x>=0&&r.y>=0&&r.right<=innerWidth&&r.bottom<=innerHeight,sanitized:f.srcdoc};})()')
assert probe['sandbox']=='' and probe['opaqueOrigin'] and probe['noParentDocument'] and probe['messages']==[] and probe['focused'] and probe['contained'],probe
assert 'onclick' not in probe['sanitized'] and '<script' not in probe['sanitized'] and 'https://example.invalid' not in probe['sanitized']
screenshot('ag1-static-preview-light')
(ROOT/'implementation/evidence/P04/native-reconnect-preview.json').write_text(json.dumps({'pending_binding_restored':True,'same_worker_after_reload':True,'approved_once':True,'preview':probe,'capability':'Static HTML only; interactive preview is deferred to the P11 platform gate.'},indent=2))
print('Native pending approval restored, one approved write completed, static preview origin/focus/navigation isolation passed.')
