import hashlib, http.client, http.server, json, os, shutil, signal, socket, subprocess, threading, time
from pathlib import Path

ROOT=Path('/home/alex/Projects/AG1');E=Path(__file__).parent;W=ROOT/'.local/bounded-comparison-20260913/opencode'
plan=json.loads((E/'plan.json').read_text());cfg=json.loads((ROOT/'.local/p14-long-horizon/scenario-validation-20260913/candidate/config.json').read_text())
capture=json.loads(Path(cfg['runtime']['capture']['path']).read_text());launch=next(m['expected'] for m in capture['request']['models'] if m['model_id']==cfg['selected'])
common={'cache_prompt':True,'temperature':0.7,'top_p':0.95,'top_k':20,'min_p':0,'presence_penalty':0,'repetition_penalty':1,'max_tokens':65536,'max_completion_tokens':65536,'chat_template_kwargs':{'enable_thinking':True}}
state={'stage':'setup','requests':0};lock=threading.Lock()
class Proxy(http.server.BaseHTTPRequestHandler):
 protocol_version='HTTP/1.1'
 def log_message(self,*args):pass
 def do_GET(self):self.forward(None)
 def do_POST(self):
  raw=self.rfile.read(int(self.headers.get('Content-Length',0)));body=json.loads(raw)
  if self.path.endswith('/chat/completions'):
   original={k:body.get(k) for k in common};body.update(common)
   with lock:
    state['requests']+=1
    record={'stage':state['stage'],'request':state['requests'],'time':time.time(),'incoming_sampling':original,'effective_sampling':common,'messages':len(body.get('messages',[])),'request_bytes':len(raw),'tools':[x.get('function',{}).get('name') for x in body.get('tools',[])]}
    with (E/'opencode-requests.jsonl').open('a') as f:f.write(json.dumps(record)+'\n')
   if state['requests']>32:self.send_error(429,'Stage generation limit reached');return
  self.forward(json.dumps(body).encode())
 def forward(self,body):
  upstream=http.client.HTTPConnection('127.0.0.1',18080,timeout=300)
  try:
   upstream.request('POST' if body is not None else 'GET',self.path,body,{'Content-Type':'application/json'});res=upstream.getresponse()
   self.send_response(res.status);self.send_header('Content-Type',res.getheader('Content-Type','application/json'));self.send_header('Connection','close');self.end_headers();self.close_connection=True
   while chunk:=res.read1(4096):self.wfile.write(chunk);self.wfile.flush()
  except (BrokenPipeError,ConnectionResetError):pass
  except Exception as ex:
   with (E/'opencode-proxy-errors.log').open('a') as f:f.write(type(ex).__name__+': '+str(ex)+'\n')
  finally:upstream.close()

def stop(child):
 if child and child.poll() is None:
  os.killpg(child.pid,signal.SIGTERM)
  try:child.wait(timeout=20)
  except subprocess.TimeoutExpired:os.killpg(child.pid,signal.SIGKILL);child.wait(timeout=5)

processes=subprocess.check_output(['ps','-eo','comm'],text=True).splitlines()
assert 'llama-server' not in processes and 'ag1-desktop' not in processes,'AG1 must finish and relinquish GPU first'
with socket.socket() as sock:assert sock.connect_ex(('127.0.0.1',18080))!=0,'Preserved runtime port already occupied'
env={k:v for k,v in os.environ.items() if not k.startswith(('AG1_','OPENCODE_'))}
for kind in ['DATA','CONFIG','CACHE','STATE']:
 p=W/('xdg-'+kind.lower());p.mkdir(exist_ok=True);env['XDG_'+kind+'_HOME']=str(p)
config={'$schema':'https://opencode.ai/config.json','model':'llamacpp/'+cfg['selected'],'small_model':'llamacpp/'+cfg['selected'],'share':'disabled','autoupdate':False,'enabled_providers':['llamacpp'],'provider':{'llamacpp':{'npm':'@ai-sdk/openai-compatible','name':'Preserved local 35B','options':{'baseURL':'http://127.0.0.1:18081/v1','apiKey':'local','timeout':300000},'models':{cfg['selected']:{'name':cfg['selected'],'reasoning':True,'interleaved':{'field':'reasoning_content'},'tool_call':True,'limit':{'context':90000,'output':65536}}}}},'permission':{'*':'allow','external_directory':'deny'},'agent':{'build':{'maxSteps':32}}}
(W/'opencode.json').write_text(json.dumps(config,indent=2));env.update(OPENCODE_CONFIG=str(W/'opencode.json'),OPENCODE_ENABLE_EXA='1',OPENCODE_DISABLE_AUTOUPDATE='1',OPENCODE_DISABLE_SHARE='1',NO_COLOR='1')
(E/'opencode-isolation.json').write_text(json.dumps({'config':str(W/'opencode.json'),'context':90000,'output':65536,'normalized_sampling':common,'launch_command_sha256':hashlib.sha256(json.dumps({'command':launch['command'],'args':launch['args'],'environment':launch['environment']},sort_keys=True).encode()).hexdigest(),'server_plan':'Exact captured 35B command, arguments and environment. No profile edits.','config_note':'Original user OpenCode configuration is untouched; isolated config matches AG1 context/output limits.','search':'OpenCode native Exa enabled; AG1 native DuckDuckGo. Different services are scored explicitly.'},indent=2))
server=None;proxy=None;client=None;stages=[]
try:
 server=subprocess.Popen([launch['command'],*launch['args']],env=launch['environment'],cwd=ROOT,stdout=(E/'opencode-llama.log').open('x'),stderr=subprocess.STDOUT,start_new_session=True)
 start=time.monotonic()
 while True:
  assert server.poll() is None,'Owned llama server exited'
  try:
   c=http.client.HTTPConnection('127.0.0.1',18080,timeout=1);c.request('GET','/health');r=c.getresponse();r.read();c.close()
   if r.status==200:break
  except OSError:pass
  if time.monotonic()-start>120:raise TimeoutError('Owned model load deadline')
  time.sleep(.5)
 (E/'opencode-runtime-ready.json').write_text(json.dumps({'pid':server.pid,'load_seconds':time.monotonic()-start,'ready_time':time.time()},indent=2))
 proxy=http.server.ThreadingHTTPServer(('127.0.0.1',18081),Proxy);threading.Thread(target=proxy.serve_forever,daemon=True).start()
 coding_session=None
 for task in plan['cases'][:2]:
  state.update(stage=task['id'],requests=0);project=W/('web' if task['id']=='web' else 'coding');out=E/('opencode-'+task['id']);out.mkdir()
  command=['/usr/bin/bwrap','--die-with-parent','--unshare-pid','--ro-bind','/','/','--proc','/proc','--dev-bind','/dev','/dev','--bind',str(W),str(W),'--tmpfs','/tmp','--','/usr/bin/opencode','run','--pure','--auto','--format','json','--thinking','--model',config['model'],'--dir',str(project)]
  if task['id']=='followup':
   if not coding_session:raise RuntimeError('No coding session for follow-up')
   command.extend(['--session',coding_session])
  else:command.extend(['--title','Bounded comparison '+task['id']])
  command.append(task['prompt']);began=time.monotonic();timedout=False
  with (out/'events.jsonl').open('x') as stdout,(out/'stderr.log').open('x') as stderr:
   client=subprocess.Popen(command,env=env,cwd=project,stdout=stdout,stderr=stderr,start_new_session=True)
   try:client.wait(timeout=task['limit_seconds'])
   except subprocess.TimeoutExpired:timedout=True;stop(client)
  events=[]
  for line in (out/'events.jsonl').read_text().splitlines():
   try:events.append(json.loads(line))
   except json.JSONDecodeError:pass
  sessions=[x.get('sessionID') for x in events if x.get('sessionID')]
  if task['id']=='coding' and sessions:coding_session=sessions[-1]
  summary={'id':task['id'],'exit_code':client.returncode,'timeout':timedout,'seconds':time.monotonic()-began,'requests':state['requests'],'session':sessions[-1] if sessions else None,'event_types':{t:sum(x.get('type')==t for x in events) for t in {x.get('type') for x in events}},'completed':not timedout and client.returncode==0}
  if task['id']!='web':shutil.copytree(project,out/'workspace')
  (out/'result.json').write_text(json.dumps(summary,indent=2));stages.append(summary);print(json.dumps(summary),flush=True)
  (E/'opencode-stages.json').write_text(json.dumps(stages,indent=2))
  if timedout or client.returncode!=0:break
finally:
 stop(client)
 if proxy:proxy.shutdown();proxy.server_close()
 stop(server)
 (E/'opencode-cleanup.json').write_text(json.dumps({'server_exit':server.poll() if server else None,'client_exit':client.poll() if client else None,'finished':time.time()},indent=2))
