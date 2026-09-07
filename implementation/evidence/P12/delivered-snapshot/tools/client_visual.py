#!/usr/bin/env python3
"""Owned Linux WebKit P12 fixture. No model or existing display is contacted."""
from __future__ import annotations
import argparse,hashlib,http.server,json,os,signal,shutil,socket,struct,subprocess,sys,tempfile,threading,time
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
sys.path.insert(0,str(ROOT/'tools/probes'))
import desktop_visual as ui
class Provider(http.server.BaseHTTPRequestHandler):
    protocol_version='HTTP/1.1'
    requests=[]
    completed=[]
    retried=False
    def log_message(self,*args):pass
    def do_POST(self):
        size=int(self.headers.get('Content-Length','0'))
        if size>4*1024*1024:raise RuntimeError('oversize request')
        body=json.loads(self.rfile.read(size));self.requests.append(body)
        last=body['messages'][-1]['content'];last=last if isinstance(last,str) else ' '.join(b.get('text','') for b in last)
        if 'RETRY' in last and not type(self).retried:
            type(self).retried=True
            payload=b'{"error":{"message":"Controlled retry fixture"}}';self.send_response(429);self.send_header('Content-Type','application/json');self.send_header('Content-Length',str(len(payload)));self.send_header('Retry-After','1');self.send_header('Connection','close');self.end_headers();self.wfile.write(payload);self.close_connection=True;return
        if 'FAILURE' in last:
            payload=json.dumps({'error':{'message':'Deliberate isolated provider failure'}}).encode();self.send_response(400);self.send_header('Content-Type','application/json');self.send_header('Content-Length',str(len(payload)));self.send_header('Connection','close');self.end_headers();self.wfile.write(payload);self.close_connection=True;return
        self.send_response(200);self.send_header('Content-Type','text/event-stream');self.send_header('Connection','close');self.end_headers()
        count=100 if 'LATENCY' in last else 30 if 'FREEZE' in last else 10
        delay=.03 if 'LATENCY' in last else .2
        try:
            for i in range(count):
                text=f'⟦{int(time.time()*1000)}:{i}⟧ ' if 'LATENCY' in last else f'Native fixture result {i}. '
                event={'choices':[{'delta':{'content':text},'finish_reason':None}]}
                self.wfile.write(('data: '+json.dumps(event)+'\n\n').encode());self.wfile.flush();time.sleep(delay)
            self.wfile.write(('data: '+json.dumps({'choices':[{'delta':{},'finish_reason':'stop'}],'usage':{'prompt_tokens':1000,'completion_tokens':count}})+'\n\ndata: [DONE]\n\n').encode());self.wfile.flush();self.completed.append({'prompt':last,'time_ms':int(time.time()*1000)})
        except (BrokenPipeError,ConnectionResetError):pass
        self.close_connection=True

def terminate(process):
    if process and process.poll() is None:
        os.killpg(process.pid,signal.SIGTERM)
        try:process.wait(timeout=5)
        except subprocess.TimeoutExpired:os.killpg(process.pid,signal.SIGKILL);process.wait(timeout=5)

def main():
    parser=argparse.ArgumentParser();parser.add_argument('--output',type=Path);parser.add_argument('--quick',action='store_true');args=parser.parse_args()
    base=ROOT/'.local/p12-visual';base.mkdir(parents=True,exist_ok=True)
    work=Path(tempfile.mkdtemp(prefix='run-',dir=base));print(f'Owned fixture: {work}',flush=True)
    server=http.server.ThreadingHTTPServer(('127.0.0.1',0),Provider);threading.Thread(target=server.serve_forever,daemon=True).start()
    config={'models':[{'provider':'fixture','id':'initial','api':'openai-completions','baseUrl':f'http://127.0.0.1:{server.server_port}','contextWindow':262144,'maxTokens':4096,'input':['text'],'reasoning':False}], 'selected':'initial','credential_env':None,'system_prompt':'Owned P12 fake provider fixture.','fixture':True}
    conflict=dict(config['models'][0],id='conflicting-runtime',runtimeConflict='Pinned fixture binary mismatch; no fallback')
    config['models'].append(conflict)
    config['tooling']={'process_host':str(ROOT/'product/target/debug/ag1-process-host'),'git':'/usr/bin/git','rg':shutil.which('rg'),'fd':'/usr/bin/fd','bubblewrap':'/usr/bin/bwrap','targets':{'node':None,'npm':None,'cargo':None,'rustc':None,'godot':None,'mounts':[]}}
    config_path=work/'host.json';config_path.write_text(json.dumps(config));config_path.chmod(0o600)
    workspace=work/'workspace';state=work/'desktop';private=state/'native'/hashlib.sha256(str(workspace).encode()).hexdigest()
    subprocess.run([str(ROOT/'product/target/debug/examples/client-fixture-seed'),str(workspace),str(private),str(config_path)],check=True,cwd=ROOT)
    (state/'catalog.json').write_text(json.dumps({'projects':[],'theme':'light'}))
    (workspace/'hello.txt').write_text('Reviewable fixture file\n')
    display=next(n for n in range(100,500) if not Path(f'/tmp/.X{n}-lock').exists() and not Path(f'/tmp/.X11-unix/X{n}').exists())
    cookie=os.urandom(16);auth=work/'display.auth';auth.write_bytes(struct.pack('>H',65535)+b''.join(struct.pack('>H',len(v))+v for v in [b'',str(display).encode(),b'MIT-MAGIC-COOKIE-1',cookie]));auth.chmod(0o600)
    xvfb=desktop=None
    report={'scope':'Owned fake loopback provider, native Linux Tauri/WebKit, private authenticated Xvfb; no live inference or llama tuning','work':str(work.relative_to(ROOT)),'cases':{},'screenshots':[]}
    def capture(name):
        dark=name.endswith('-dark')
        ui.wait('document.documentElement.classList.contains("dark")==='+str(dark).lower());time.sleep(.2)
        path=ui.screenshot(name);report['screenshots'].append({'name':name,'path':str(Path(path).relative_to(ROOT)) if isinstance(path,str) else str((work/(name+'.png')).relative_to(ROOT))})
    try:
        xvfb=subprocess.Popen([str(ROOT/'.local/p11/xvfb/usr/bin/Xvfb'),f':{display}','-screen','0','1440x900x24','-nolisten','tcp','-auth',str(auth)],stdout=open(work/'xvfb.log','w'),stderr=subprocess.STDOUT,start_new_session=True)
        end=time.monotonic()+10
        while not Path(f'/tmp/.X11-unix/X{display}').exists():
            if time.monotonic()>end:raise RuntimeError('private Xvfb startup failed')
            time.sleep(.05)
        env=dict(os.environ,DISPLAY=f':{display}',XAUTHORITY=str(auth),GDK_BACKEND='x11',LIBGL_ALWAYS_SOFTWARE='1',WEBKIT_DISABLE_DMABUF_RENDERER='1',AG1_NATIVE_CONFIG=str(config_path),AG1_DESKTOP_STATE=str(state),AG1_FIXTURE_PROVIDER='1',AG1_VISUAL_SOCKET=str(work/'control.sock'))
        desktop=subprocess.Popen([str(ROOT/'product/target/debug/ag1-desktop')],cwd=ROOT,env=env,stdout=open(work/'desktop.log','w'),stderr=subprocess.STDOUT,start_new_session=True)
        end=time.monotonic()+20
        while not (work/'control.sock').exists():
            if desktop.poll() is not None or time.monotonic()>end:raise RuntimeError('native desktop startup failed; '+str(work/'desktop.log'))
            time.sleep(.05)
        ui.SOCKET=work/'control.sock';ui.wait('document.body.innerText.includes("What will you work on?")')
        dimensions=ui.js('({width:innerWidth,height:innerHeight,dpr:devicePixelRatio})');assert dimensions=={'width':1440,'height':900,'dpr':1},dimensions;report['viewport']=dimensions
        capture('home-light');ui.click('Toggle theme');capture('home-dark');ui.click('Toggle theme')
        ui.js('document.dispatchEvent(new KeyboardEvent("keydown",{key:"o",ctrlKey:true,bubbles:true}));true');ui.wait('!!document.querySelector(".dialog input")');ui.text('input[aria-label="Project folder"]',str(workspace));capture('dialog-light');ui.js('document.querySelector(".dialog form").requestSubmit();true')
        ui.wait('document.body.innerText.includes("1000 messages loaded") || document.body.innerText.includes("of 1000 messages loaded")',20)
        capture('chat-light');ui.click('Toggle theme');capture('chat-dark');ui.click('Toggle theme')
        while ui.js('[...document.querySelectorAll("button")].some(b=>b.textContent==="Load earlier messages")'):
            ui.click('Load earlier messages');ui.wait('![...document.querySelectorAll("button")].some(b=>b.textContent==="Loading history…")')
        assert ui.js('document.body.innerText.includes("1000 of 1000 messages loaded")')
        scroll=ui.js('(()=>{const s=document.querySelector(".native-conversation [data-virtuoso-scroller]");s.scrollTop=0;return !!s;})()');assert scroll
        ui.wait('document.body.innerText.includes("Fixture message 0:")')
        report['cases']['1000_message_scroll']={'passed':True,'rendered_articles':ui.js('document.querySelectorAll(".native-conversation article").length')};capture('history-start-light')
        ui.js('(()=>{const s=document.querySelector(".native-conversation [data-virtuoso-scroller]");s.scrollTop=s.scrollHeight;return true;})()')
        ui.click('Files');ui.click('hello.txt');ui.wait('!!document.querySelector(".monaco-editor")');capture('files-light')
        ui.js('(()=>{const input=document.querySelector(".monaco-editor textarea");input.focus();document.execCommand("insertText",false,"UI edit: ");return true;})()')
        ui.wait('!![...document.querySelectorAll("[aria-label]")].some(n=>n.getAttribute("aria-label")==="Unsaved changes")');capture('files-dirty-light')
        (workspace/'hello.txt').write_text('External fixture change\n');ui.click('Save')
        ui.wait('document.body.innerText.includes("Conflict")');assert ui.js('!![...document.querySelectorAll("[aria-label]")].some(n=>n.getAttribute("aria-label")==="Unsaved changes")')
        assert (workspace/'hello.txt').read_text()=='External fixture change\n';report['cases']['failed_save_retains_dirty_buffer']={'passed':True};capture('files-conflict-light');ui.click('Discard');ui.click('Chat')
        ui.js('document.querySelector(".context-strip").click();true');ui.wait('!!document.querySelector(".details-drawer")');capture('task-light')
        ui.click('Plans');ui.wait('document.body.innerText.includes("Review fixture changes")');capture('plans-light')
        ui.click('accept');ui.click('Refresh plans');ui.wait('document.body.innerText.includes("accepted")');report['cases']['native_plan_review']={'passed':True}
        ui.click('Context');capture('context-light')
        ui.click('Artifacts & memory');capture('artifacts-light')
        ui.click('Activity');capture('activity-light')
        ui.click('History');ui.wait('document.body.innerText.includes("Session history and branches")');capture('history-light')
        ui.click('Models');ui.wait('document.body.innerText.includes("Pinned fixture binary mismatch")');assert ui.js('[...document.querySelectorAll("option")].some(o=>o.disabled&&o.textContent.includes("conflicting-runtime"))');report['cases']['model_conflict_no_retune']={'passed':True};capture('models-conflict-light')
        ui.click('Runtime');ui.wait('document.body.innerText.includes("Owned project preview")');capture('runtime-light')
        ui.click('Show terminal');ui.click('Start terminal');ui.wait('!!document.querySelector(".terminal-output")&&document.querySelector(".terminal-output").textContent.includes("ag1>")');capture('terminal-light');ui.click('Stop terminal');ui.click('Hide terminal')
        ui.click('Resources');ui.wait('document.body.innerText.includes("Create a skill")');capture('resources-light')
        ui.text('.details-drawer input','visual-review');ui.text('.details-drawer textarea','Review this isolated fixture')
        ui.click('Create skill');ui.wait('document.body.innerText.includes("Allow this operation")');capture('approval-light');ui.click('Allow this operation');ui.wait('!document.body.innerText.includes("Allow this operation")');assert (workspace/'.ag1/skills/visual-review/SKILL.md').is_file();report['cases']['native_resource_approval']={'passed':True}
        ui.click('Close details')
        ui.text('textarea[aria-label="Message AG1"]','FAILURE provider fixture');ui.click('Send message');ui.wait('!!document.querySelector(".error")&&!document.querySelector(".streaming")',20);capture('provider-failure-light');report['cases']['provider_failure_visible']={'passed':True}
        ui.js('window.__p12Lags=[];window.__p12Seen=new Set();window.__p12Observer=new MutationObserver(()=>{const text=document.querySelector(".streaming pre")?.textContent??"";for(const m of text.matchAll(/⟦(\\d+):(\\d+)⟧/g)){if(!window.__p12Seen.has(m[0])){window.__p12Seen.add(m[0]);window.__p12Lags.push(Date.now()-Number(m[1]));}}});window.__p12Observer.observe(document.body,{childList:true,subtree:true,characterData:true});true')
        ui.text('textarea[aria-label="Message AG1"]','LATENCY controlled display fixture');ui.click('Send message')
        ui.wait('!!document.querySelector(".streaming")');capture('streaming-light')
        ui.wait('window.__p12Lags.length>=95',20);ui.wait('!document.querySelector(".streaming")',20)
        lags=sorted(ui.js('window.__p12Lags'));report['latency']={'samples':len(lags),'p50_ms':lags[len(lags)//2],'p95_ms':lags[int((len(lags)-1)*.95)],'max_ms':max(lags),'limit_ms':50}
        report['cases']['display_latency']={'passed':report['latency']['p95_ms']<50};print('Display latency: '+json.dumps(report['latency']),flush=True)
        if not args.quick:
            ui.text('textarea[aria-label="Message AG1"]','FREEZE renderer fixture');ui.click('Send message');ui.wait('!!document.querySelector(".streaming")')
            began=int(time.time()*1000);ui.js('setTimeout(()=>{const end=Date.now()+30000;while(Date.now()<end){}},100);true');print('Freezing only the owned renderer for 30 seconds',flush=True);time.sleep(31)
            ui.wait('!document.querySelector(".streaming")',10)
            completed=next(x for x in Provider.completed if 'FREEZE' in x['prompt']);assert completed['time_ms']<began+30000
            report['cases']['frozen_renderer_resnapshot']={'passed':True,'frozen_ms':30000,'provider_completed_ms':completed['time_ms']-began};capture('resnapshot-light')
        ui.text('textarea[aria-label="Message AG1"]','RETRY controlled fixture');ui.click('Send message')
        ui.js('document.querySelector(".context-strip").click();true');ui.click('Activity');ui.wait('document.querySelector(".details-drawer").innerText.toLowerCase().includes("retry")',20);capture('retry-light');ui.wait('!document.querySelector(".streaming")',20);report['cases']['native_retry_visible']={'passed':True}
        ui.click('Context');ui.click('Compact context');ui.wait('document.querySelector(".context-strip").innerText.includes("streaming")',20);capture('compaction-light');ui.wait('document.querySelector(".context-strip").innerText.includes("idle")',30);assert not ui.js('!!document.querySelector(".error")');report['cases']['native_compaction']={'passed':True};ui.click('Close details')
        ui.text('textarea[aria-label="Message AG1"]','/new');ui.click('Send message');ui.wait('document.body.innerText.includes("Ready when you are.")',20);capture('chat-empty-light');report['cases']['native_new_session_empty']={'passed':True}
        report['requests']=len(Provider.requests);report['status']='passed' if all(c['passed'] for c in report['cases'].values()) else 'failed'
    finally:
        if desktop and desktop.poll() is None:
            try:
                (work/'last-ui.json').write_text(json.dumps(ui.js('({text:document.body.innerText.slice(-10000)})'),indent=2));ui.screenshot('last-ui')
            except Exception:pass
        terminate(desktop);terminate(xvfb);server.shutdown();server.server_close()
        (work/'report.json').write_text(json.dumps(report,indent=2)+'\n')
        if args.output:args.output.parent.mkdir(parents=True,exist_ok=True);args.output.write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps(report,indent=2),flush=True)
    if report.get('status')!='passed':raise SystemExit(1)
if __name__=='__main__':main()
