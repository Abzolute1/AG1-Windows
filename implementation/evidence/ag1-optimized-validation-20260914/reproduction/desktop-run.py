import argparse,hashlib,json,os,signal,struct,subprocess,sys,time,uuid
from pathlib import Path
ROOT=Path('/home/alex/Projects/AG1');R=ROOT/'.local/ag1-optimized-validation-20260914';W=ROOT/'.local/p14-long-horizon/ag1perf-20260914';E=ROOT/'implementation/evidence/ag1-optimized-validation-20260914'
sys.path.insert(0,str(ROOT/'tools/probes'));import desktop_visual as ui
p=argparse.ArgumentParser();p.add_argument('arm',choices=['original','candidate']);p.add_argument('--label');p.add_argument('--lifecycle',action='store_true');a=p.parse_args();arm=a.arm;label=a.label or arm;d=W/label;o=E/label;o.mkdir();r=R/label

def save(name,x): (o/name).write_text(json.dumps(x,indent=2)+'\n')
def view():return ui.request({'native_view':True})['view']
def command(c):
 s=view()['snapshot'];env={'version':s['version'],'id':'ag1perf-'+uuid.uuid4().hex,'scope':s['scope'],'expected_revision':s['revision'],'command':c}
 return ui.async_js('return await window.__TAURI_INTERNALS__.invoke("bridge_command",{envelope:'+json.dumps(env)+'});')
def waitview(test,timeout=180):
 start=time.monotonic()
 while time.monotonic()-start<timeout:
  v=view()
  if v.get('error'):raise RuntimeError(v['error'])
  if test(v):return v
  time.sleep(.5)
 raise TimeoutError('native state condition')
def modelpids():
 ids=subprocess.check_output(['nvidia-smi','--query-compute-apps=pid','--format=csv,noheader'],text=True);found=[]
 for x in ids.splitlines():
  try:pid=int(x);path=Path(f'/proc/{pid}/exe').resolve(strict=True)
  except (ValueError,OSError):continue
  if path.name=='llama-server':found.append((pid,str(path)))
  elif path.name.lower() not in ['discord','chrome','chromium','electron','codex']:raise RuntimeError('Unexpected GPU process '+path.name)
 return found
assert not modelpids(),'Another inference process is already active'
# Deterministic fixture independent of the agent's own checks.
fixture={f'svc-{i:03d}':{'timeout_ms':300+i%11*50,'retry_budget':i%4,'owner':f'team-{i%13}'} for i in range(120)}
initial=json.dumps(fixture,indent=2)+'\n';(d/'workspace/services.json').write_text(initial);expected=initial.replace('"svc-047": {\n    "timeout_ms": 450,','"svc-047": {\n    "timeout_ms": 875,');assert expected!=initial
(o/'services-before.json').write_text(initial)
checker='''import json\nfrom pathlib import Path\np=Path(__file__).with_name('services.json')\nx=json.loads(p.read_text())\nassert len(x)==120\nfor i in range(120):\n expected={'timeout_ms':875 if i==47 else 300+i%11*50,'retry_budget':i%4,'owner':f'team-{i%13}'}\n assert x[f'svc-{i:03d}']==expected,(i,x[f'svc-{i:03d}'],expected)\nprint('120 service records verified')\n'''
(d/'workspace/verify.py').write_text(checker);checker_hash=hashlib.sha256(checker.encode()).hexdigest()
# Authenticated private display; no capture of the user's screen.
display=next(n for n in range(350,500) if not Path(f'/tmp/.X{n}-lock').exists() and not Path(f'/tmp/.X11-unix/X{n}').exists());auth=r/'display.auth';auth.write_bytes(struct.pack('>H',65535)+b''.join(struct.pack('>H',len(v))+v for v in [b'',str(display).encode(),b'MIT-MAGIC-COOKIE-1',os.urandom(16)]));auth.chmod(0o600)
ui.SOCKET=r/'control.sock';assert len(str(ui.SOCKET).encode())<108
baseenv={k:v for k,v in os.environ.items() if not k.startswith('AG1_')};env={**baseenv,'DISPLAY':f':{display}','XAUTHORITY':str(auth),'GDK_BACKEND':'x11','LIBGL_ALWAYS_SOFTWARE':'1','WEBKIT_DISABLE_DMABUF_RENDERER':'1','AG1_NATIVE_CONFIG':str(d/'config.json'),'AG1_RELEASE_WORKSPACE':str(d/'workspace'),'AG1_DESKTOP_STATE':str(d/'desktop'),'AG1_VISUAL_SOCKET':str(ui.SOCKET),'AG1_INSTALL_ROOT':str(ROOT)}
for k in ['DATA','CONFIG','CACHE']:
 path=d/('xdg-'+k.lower());path.mkdir(mode=0o700);env['XDG_'+k+'_HOME']=str(path)
prefix=['/usr/bin/bwrap','--die-with-parent','--unshare-pid','--ro-bind','/','/','--proc','/proc','--dev-bind','/dev','/dev','--bind',str(R),str(R),'--bind',str(W),str(W),'--bind',str(R/'authority'),'/home/alex/.local/state/ag1/authority-v1','--tmpfs','/tmp','--ro-bind','/tmp/.X11-unix','/tmp/.X11-unix','--']
xvfb=app=None;report={'arm':arm,'scope':'Actual AG1 Tauri/WebKit desktop, current source with control broker enabled, real native runtime and tools. No mock provider; no screenshots.','tests':[]}
def submit(label,prompt,timeout=240):
 before=view();oldmessages=len(before['details']['messages']);old_ids={m['id'] for m in before['details']['messages']};start=time.monotonic();ui.text('textarea[aria-label="Message AG1"]',prompt)
 ui.wait("!!document.querySelector('button[aria-label=\"Send message\"]')&&!document.querySelector('button[aria-label=\"Send message\"]').disabled",20)
 ui.js('(()=>{let t=document.querySelector(\'textarea[aria-label="Message AG1"]\');t.closest("form").requestSubmit();return true;})()')
 observations=[];first=None;active=False;timedout=False
 while True:
  v=view();elapsed=time.monotonic()-start
  if v['snapshot']['status']!='idle':active=True
  messages=v['details']['messages'];new=[m for m in messages if m['id'] not in old_ids]
  if first is None and any(m['role']=='assistant' and m.get('blocks') for m in new):first=elapsed
  observations.append({'elapsed':elapsed,'status':v['snapshot']['status'],'messages':len(messages),'progress':v.get('progress'),'error':v.get('error'),'pending':len(v['snapshot']['pending'])})
  if v.get('error'):break
  if active and v['snapshot']['status']=='idle':break
  if v['snapshot']['pending']:
   approvals=[]
   for question in v['snapshot']['pending']:
    if question['kind']!='approval':raise RuntimeError('Unattended test reached a non-approval question')
    details=json.loads(question['title']);tool=details.get('toolName','');description=question['title']
    allowed=(tool in ['edit','write'] and 'services.json' in description) or (tool in ['bash','shell','runCommand','exec'] and 'verify.py' in description)
    if not allowed:
     save(label+'-unexpected-approval.json',v);raise RuntimeError('Approval outside bounded fixture task: '+tool)
    reply=command({'kind':'approve','binding':question['binding'],'allow':True})
    approvals.append({'question':question,'reply':reply})
   save(label+'-approvals-'+str(len(observations))+'.json',approvals)
   time.sleep(.5);continue
  if elapsed>timeout:
   timedout=True
   if v['snapshot'].get('active_operation'):command({'kind':'cancel','operation':v['snapshot']['active_operation']})
   v=waitview(lambda x:x['snapshot']['status']=='idle',45);break
  time.sleep(1)
 save(label+'-view.json',v);save(label+'-observations.json',observations);(o/(label+'-visible.txt')).write_text(ui.js('document.body.innerText'))
 result={'name':label,'elapsed_seconds':time.monotonic()-start,'first_assistant_block_seconds':first,'timed_out':timedout,'terminal':v['snapshot'].get('last_terminal'),'error':v.get('error'),'pending':len(v['snapshot']['pending']),'new_message_ids_in_final_view':len([m for m in v['details']['messages'] if m['id'] not in old_ids])}
 report['tests'].append(result);save('result.json',report);print(json.dumps(result),flush=True);return v
try:
 with (r/'xvfb.log').open('w')as log:xvfb=subprocess.Popen([str(ROOT/'.local/p11/xvfb/usr/bin/Xvfb'),f':{display}','-screen','0','1440x900x24','-nolisten','tcp','-auth',str(auth)],stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
 end=time.monotonic()+15
 while not Path(f'/tmp/.X11-unix/X{display}').exists():
  assert xvfb.poll()is None and time.monotonic()<end;time.sleep(.1)
 with (r/'desktop.log').open('w')as log:app=subprocess.Popen(prefix+[str(R/'app/ag1-desktop')],cwd=ROOT,env=env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
 end=time.monotonic()+90
 while not ui.SOCKET.exists():
  if app.poll()is not None or time.monotonic()>end:raise RuntimeError('Desktop/control socket did not start; see private desktop.log')
  time.sleep(.1)
 ui.wait('!!document.querySelector(".workspace-pill")',45)
 ui.js('document.dispatchEvent(new KeyboardEvent("keydown",{key:"o",ctrlKey:true,bubbles:true}));true');ui.wait('!!document.querySelector(\'input[aria-label="Project folder"]\')');ui.text('input[aria-label="Project folder"]',str(d/'workspace'));ui.js('document.querySelector(".dialog form").requestSubmit();true')
 ui.wait('!!document.querySelector(\'textarea[aria-label="Message AG1"]\')',120)
 v=view();assert v['snapshot']['workspace_path']==str(d/'workspace'),v['snapshot']['workspace_path'];save('initial-view.json',v)
 ui.text('textarea[aria-label="Message AG1"]','Keep this draft while loading.');ui.wait('!!document.querySelector(".model-load-button")&&!document.querySelector(".model-load-button").disabled',30);start=time.monotonic();ui.js('document.querySelector(".model-load-button").click();true');states=[]
 while time.monotonic()-start<180:
  text=ui.js('document.querySelector(".model-load-control")?.innerText??""');states.append({'elapsed':time.monotonic()-start,'text':text})
  if 'Loaded' in text:break
  if 'failed' in text.lower():raise RuntimeError(text)
  time.sleep(.5)
 else:raise TimeoutError('UI manual model load')
 assert ui.js('document.querySelector(\'textarea[aria-label="Message AG1"]\').value')=='Keep this draft while loading.'
 pid,exe=modelpids()[0];assert len(modelpids())==1;maps=Path(f'/proc/{pid}/maps').read_text();(o/'backend-maps.txt').write_text(maps)
 argv=Path(f'/proc/{pid}/cmdline').read_bytes().split(b'\0');save('actual-backend.json',{'pid':pid,'executable':exe,'args':[v.decode() for v in argv if v]})
 if arm=='candidate':assert 'bundle-v1/bin/libggml-cuda' in maps and '/llama-cpp-turboquant/build/' not in maps
 report['manual_load']={'seconds':time.monotonic()-start,'draft_preserved':True,'no_messages_submitted':view()['snapshot']['message_count']==0,'states':states,'backend_pid':pid};save('result.json',report);print(json.dumps({'arm':arm,'manual_load_seconds':report['manual_load']['seconds']}),flush=True)
 if not a.lifecycle:
  submit('chat','Answer directly without using tools. Return exactly this JSON object with the computed integer: {"answer": 17 multiplied by 19}.',120)
  # Start another actual conversation through the app and verify that the model remains resident.
  if not ui.js('!!document.querySelector(".new-chat")'):
   ui.js('(()=>{let b=[...document.querySelectorAll("button")].find(b=>b.getAttribute("aria-label")==="Show sidebar");if(b)b.click();return true;})()')
  ui.wait('!!document.querySelector(".new-chat")',10);previous=view()['snapshot']['scope']['session'];ui.js('document.querySelector(".new-chat").click();true');waitview(lambda x:x['snapshot']['scope']['session']!=previous,30)
  report['new_chat_reuses_backend']=modelpids()==[(pid,exe)];save('result.json',report)
  submit('edit','In the attached project, read services.json. Change only svc-047 timeout_ms to 875. Preserve every other value and the existing formatting. Do not modify verify.py. Run python verify.py to validate the edit, then report the result briefly. Use the actual file and execution tools; do not merely describe the change.',240)
  actual=(d/'workspace/services.json').read_text();report['edit_exact_pass']=actual==expected;report['checker_preserved']=hashlib.sha256((d/'workspace/verify.py').read_bytes()).hexdigest()==checker_hash;(o/'services-after.json').write_text(actual)
  submit('web','Use webSearch once to find Python documentation for time.monotonic. Use webFetch to read a relevant official docs.python.org page from the results. Then explain in at most 100 words why monotonic time is useful for measuring elapsed time. Cite the page you actually read. Do not modify any project files.',180)
 else:
  submit('before-cancel','Answer without tools: what is 17 multiplied by 19? Return only the integer.',120)
  ui.text('textarea[aria-label="Message AG1"]','Write a detailed numbered list of 1000 distinct observations about software testing. Continue through all 1000 entries. Do not use tools or edit files.')
  ui.wait("!!document.querySelector('button[aria-label=\"Send message\"]')&&!document.querySelector('button[aria-label=\"Send message\"]').disabled",20)
  ui.js('document.querySelector("textarea").closest("form").requestSubmit();true')
  active_view=waitview(lambda x:x['snapshot'].get('active_operation') is not None,60)
  time.sleep(5)
  save('before-stop-view.json',view());ui.wait('!!document.querySelector(\'button[aria-label="Stop generation"]\')',10)
  stopped_at=time.monotonic();ui.js('document.querySelector(\'button[aria-label="Stop generation"]\').click();true')
  settled=waitview(lambda x:x['snapshot']['status']=='idle',45)
  report['stop']={'seconds_to_idle':time.monotonic()-stopped_at,'error':settled.get('error'),'pending':len(settled['snapshot']['pending']),'model_after_stop':modelpids()};save('after-stop-view.json',settled);save('result.json',report)
  submit('after-cancel','Ignore the cancelled list. Answer without tools. Return exactly the JSON object {"recovered":true,"answer":323}.',120)
  report['post_stop_healthy']=not view().get('error') and view()['snapshot']['status']=='idle'
  report['ui_health']=ui.js('({width:innerWidth,height:innerHeight,overflow:document.documentElement.scrollWidth>innerWidth+1,alerts:[...document.querySelectorAll("[role=alert]")].map(x=>x.innerText),model:document.querySelector(".model-load-control")?.innerText})')
 report['backend_survived_all_tasks']=modelpids()==[(pid,exe)];report['status']='completed';save('result.json',report)
except Exception as e:
 report.update(status='failed',error=repr(e));save('result.json',report);raise
finally:
 if app and app.poll()is None:
  try:
   ui.js('void window.__TAURI_INTERNALS__.invoke("quit");true')
  except Exception:pass
  try:app.wait(timeout=20);report['normal_app_quit']=app.returncode==0
  except Exception:report['normal_app_quit']=False
  try:os.killpg(app.pid,signal.SIGTERM)
  except ProcessLookupError:pass
  try:app.wait(timeout=20)
  except subprocess.TimeoutExpired:os.killpg(app.pid,signal.SIGKILL);app.wait()
 if xvfb and xvfb.poll()is None:
  os.killpg(xvfb.pid,signal.SIGTERM);xvfb.wait(timeout=10)
 report['remaining_model_processes']=modelpids();save('result.json',report)
