#!/usr/bin/env python3
import resource,ctypes,argparse,hashlib,json,os,signal,statistics,subprocess,threading,time,urllib.request,urllib.error
from pathlib import Path
ROOT=Path('/home/alex/Projects/llama-perf-20260914')
EVID=Path('/home/alex/Projects/AG1/implementation/evidence/llama-performance-20260914')
BASE=json.loads((EVID/'original-launch.json').read_text())
PORT=18436
URL=f'http://127.0.0.1:{PORT}'
def api(path,data=None,timeout=900):
 req=urllib.request.Request(URL+path,data=json.dumps(data).encode() if data is not None else None,headers={'Content-Type':'application/json'})
 return urllib.request.urlopen(req,timeout=timeout)
def get(path):
 with api(path,timeout=2) as r:return json.load(r)
def post(path,data):
 with api(path,data) as r:return json.load(r)
def median(xs):return statistics.median(xs) if xs else None

def build_prompts():
 p=EVID/'prompts.json'
 if p.exists():return json.loads(p.read_text())
 records=[]
 for i in range(180):
  records.append(f'Service svc-{i:03d}: region={"eu-north" if i%3==0 else "us-east"}; replicas={2+i%7}; requests={1300+i*17}; failures={i%9}; timeout_ms={300+i%11*50}; retry_budget={i%4}; owner=team-{i%13}; deploy=rev-{9000+i}; cache_ttl={30+i%5*15}. The service accepts idempotent reads and tagged writes. Retry transient errors only within the remaining deadline; apply jitter and honor Retry-After. Its last maintenance verified connection reuse and bounded queues.')
 code='''def fetch_with_retry(session, url, deadline):
    for attempt in range(5):
        response = session.post(url, timeout=30)
        if response.status_code == 429 or response.status_code >= 500:
            time.sleep(2 ** attempt)
            continue
        return response.json()
    return None
'''
 tasks={
  'short':{'messages':[{'role':'system','content':'You are a precise software performance reviewer. Answer directly without private reasoning. Use only the supplied information.'},{'role':'user','content':'Review this retry implementation. Explain deadline handling, idempotency, backoff, error propagation and connection reuse, then give corrected Python-like pseudocode. Give a detailed answer of at least 400 words.\n'+code+'\nOperational context:\n'+'\n'.join(records[:8])}]},
  'medium':{'messages':[{'role':'system','content':'You are a precise software performance reviewer. Answer directly without private reasoning. Use only the supplied information.'},{'role':'user','content':'Read this service inventory and review the retry implementation below. Compare the retry policy for svc-003, svc-021, svc-047, svc-075 and svc-089, citing their timeout and retry_budget. Explain concrete correctness fixes and show pseudocode. Give a detailed answer of at least 400 words.\nInventory:\n'+'\n'.join(records[:90])+'\nImplementation:\n'+code}]},
  'quality':{'messages':[{'role':'system','content':'Answer directly using only the supplied records. Return only a JSON object. Do not include reasoning or markdown.'},{'role':'user','content':'Return the exact replicas, timeout_ms, retry_budget, owner and deploy fields for svc-003, svc-021, svc-047, svc-075 and svc-089. Keys must be the service names.\n'+'\n'.join(records[:90])}]},
  'long':{'messages':[{'role':'system','content':'You are a precise software performance reviewer. Answer directly using the supplied information.'},{'role':'user','content':'Read all the following deployment snapshots. The last snapshot is authoritative. Summarize retry correctness risks and give concrete pseudocode fixes, in at least 400 words.\n'+('\n'.join(records)+'\n')*3+'\nReview this code:\n'+code}]}
 }
 for k,v in tasks.items():
  v['prompt']=post('/apply-template',{'messages':v['messages'],'chat_template_kwargs':{'enable_thinking':False}})['prompt']
  v['tokens']=post('/tokenize',{'content':v['prompt'],'add_special':False})['tokens']
  v['sha256']=hashlib.sha256(v['prompt'].encode()).hexdigest()
 p.write_text(json.dumps(tasks,indent=2));return tasks

def run(config):
 resource.setrlimit(resource.RLIMIT_CORE,(0,0))
 name=config['name'];out=EVID/'runs'/name;out.mkdir(parents=True,exist_ok=False)
 args=BASE['args'].copy();overrides={'--port':str(PORT),**config.get('flags',{})}
 for k,v in overrides.items():
  if k in args:
   i=args.index(k);args[i+1]=str(v)
  else:args += [k,str(v)]
 for k in config.get('remove',[]):
  if k in args:
   i=args.index(k);args.pop(i)
   if i<len(args) and not args[i].startswith('-'):args.pop(i)
 args+=config.get('extra',[])
 command=config.get('binary',BASE['command']);env=BASE['environment'].copy();env.update(config.get('environment',{}))
 if 'binary' in config:env['LD_LIBRARY_PATH']=str(Path(command).parent)
 pre=subprocess.check_output(['nvidia-smi','--query-compute-apps=pid','--format=csv,noheader'],text=True).strip()
 background=[]
 for pid in pre.splitlines():
  try: exe=Path(f'/proc/{int(pid)}/exe').resolve().name
  except (ValueError,OSError):continue
  if exe.lower() not in ['discord','chrome','chromium','electron','codex']:raise RuntimeError('Another GPU compute process is active: '+exe)
  background.append({'pid':int(pid),'executable':exe})
 (out/'background.json').write_text(json.dumps(background))
 (out/'launch.json').write_text(json.dumps({'binary':command,'args':args,'environment':env,'configuration':config},indent=2))
 monitor_stop=threading.Event();phase=['load'];samples=[];proc=None
 def monitor():
  previous=None
  nvml=ctypes.CDLL('libnvidia-ml.so.1');dev=ctypes.c_void_p()
  nvml_ok=nvml.nvmlInit_v2()==0 and nvml.nvmlDeviceGetHandleByIndex_v2(0,ctypes.byref(dev))==0
  while not monitor_stop.is_set():
   now=time.monotonic();row={'time':now,'phase':phase[0]}
   try:
    lines=subprocess.check_output(['nvidia-smi','--query-gpu=utilization.gpu,utilization.memory,memory.used,temperature.gpu,power.draw,clocks.sm,clocks.mem','--format=csv,noheader,nounits'],text=True,timeout=2).strip().split(', ')
    row.update(dict(zip(['gpu_pct','gpu_memory_pct','vram_mib','gpu_temp','gpu_watts','sm_mhz','memory_mhz'],map(float,lines))))
    if nvml_ok:
     for counter,label in [(0,'pcie_tx_kibs'),(1,'pcie_rx_kibs')]:
      value=ctypes.c_uint()
      if nvml.nvmlDeviceGetPcieThroughput(dev,counter,ctypes.byref(value))==0:row[label]=value.value
    if proc:
     stat=Path(f'/proc/{proc.pid}/stat').read_text().rsplit(')',1)[1].split();ticks=int(stat[11])+int(stat[12]);rss=int(stat[21])*os.sysconf('SC_PAGE_SIZE')/1048576
     row['rss_mib']=rss
     if previous:row['cpu_pct']=(ticks-previous[1])/os.sysconf('SC_CLK_TCK')/(now-previous[0])*100
     previous=(now,ticks)
     status=Path(f'/proc/{proc.pid}/status').read_text();row['threads']=int(next(l for l in status.splitlines() if l.startswith('Threads:')).split()[1])
   except (Exception,) as e:row['monitor_error']=type(e).__name__
   samples.append(row);monitor_stop.wait(.5)
  if nvml_ok:nvml.nvmlShutdown()
 def summary_samples(label):
  rows=[r for r in samples if r['phase']==label]
  return {k:{'median':median([r[k] for r in rows if k in r]),'max':max([r[k] for r in rows if k in r],default=None)} for k in ['gpu_pct','gpu_memory_pct','vram_mib','rss_mib','cpu_pct','gpu_temp','gpu_watts','sm_mhz','pcie_rx_kibs','pcie_tx_kibs']}
 start=time.monotonic();summary={'name':name,'config':config,'requests':[],'started_at':time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime())}
 log=(out/'server.log').open('w');t=threading.Thread(target=monitor,daemon=True)
 try:
  proc=subprocess.Popen([command]+args,env=env,cwd=ROOT,stdout=log,stderr=subprocess.STDOUT,start_new_session=True);(out/'pid').write_text(str(proc.pid));t.start()
  while True:
   if proc.poll() is not None:raise RuntimeError(f'Server exited during load: {proc.returncode}')
   try:
    if get('/health').get('status')=='ok':break
   except Exception:pass
   if time.monotonic()-start>300:raise TimeoutError('model load exceeded 300s')
   time.sleep(.2)
  summary['load_seconds']=time.monotonic()-start;phase[0]='idle';(out/'props.json').write_text(json.dumps(get('/props'),indent=2));prompts=build_prompts()
  for workload in config.get('workloads',['short','medium']):
   for repeat in range(config.get('repetitions',3)):
    phase[0]=f'{workload}-{repeat}-prefill';payload={'prompt':prompts[workload]['tokens'],'stream':True,'n_predict':config.get('n_predict',256),'temperature':0,'seed':777,'top_k':1,'top_p':1,'min_p':0,'repeat_penalty':1,'cache_prompt':False,'id_slot':0}
    if workload=='quality':payload['n_predict']=768
    started=time.monotonic();first=None;parts=[];final={};events=[]
    with api('/completion',payload) as response:
     for line in response:
      if not line.startswith(b'data: '):continue
      event=json.loads(line[6:]);events.append(event)
      if event.get('content'):
       if first is None:first=time.monotonic();phase[0]=f'{workload}-{repeat}-decode'
       parts.append(event['content'])
      if event.get('stop'):final=event
      if 'error' in event:raise RuntimeError(str(event['error']))
    ended=time.monotonic();phase[0]='idle';text=''.join(parts)
    row={'workload':workload,'repeat':repeat,'ttft_seconds':None if first is None else first-started,'wall_seconds':ended-started,'timings':final.get('timings'),'output':text,'prompt_tokens':len(prompts[workload]['tokens']),'prompt_sha256':prompts[workload]['sha256'],'prefill_resources':summary_samples(f'{workload}-{repeat}-prefill'),'decode_resources':summary_samples(f'{workload}-{repeat}-decode'),'stop_type':final.get('stop_type')}
    if not row['timings']:raise RuntimeError('Missing native timing record')
    if workload=='quality':
     expected={f'svc-{i:03d}':{'replicas':2+i%7,'timeout_ms':300+i%11*50,'retry_budget':i%4,'owner':f'team-{i%13}','deploy':f'rev-{9000+i}'} for i in [3,21,47,75,89]}
     try:row['quality_pass']=json.loads(text)==expected
     except:row['quality_pass']=False
    summary['requests'].append(row);(out/f'{workload}-{repeat}.json').write_text(json.dumps({'request':payload,'events':events,'result':row},indent=2));(out/'result.json').write_text(json.dumps(summary,indent=2))
    print(json.dumps({'run':name,'workload':workload,'rep':repeat,'ttft':row['ttft_seconds'],'timings':row['timings'],'quality':row.get('quality_pass')}),flush=True)
  summary['status']='passed'
 except Exception as e:
  summary['status']='failed';summary['error']=str(e);print(json.dumps({'run':name,'error':str(e)}),flush=True)
 finally:
  phase[0]='stopping'
  if proc and proc.poll() is None:
   os.killpg(proc.pid,signal.SIGTERM)
   try:proc.wait(timeout=15)
   except subprocess.TimeoutExpired:os.killpg(proc.pid,signal.SIGKILL);proc.wait()
  monitor_stop.set()
  if t.is_alive():t.join(timeout=4)
  log.close();summary['load_resources']=summary_samples('load');summary['peak_vram_mib']=max([s.get('vram_mib',0) for s in samples],default=0);summary['peak_rss_mib']=max([s.get('rss_mib',0) for s in samples],default=0);summary['exit_code']=proc.returncode if proc else None
  (out/'telemetry.json').write_text(json.dumps(samples));(out/'result.json').write_text(json.dumps(summary,indent=2));print(json.dumps({'run':name,'status':summary['status'],'load_seconds':summary.get('load_seconds'),'peak_vram_mib':summary['peak_vram_mib']}),flush=True)
 return summary
if __name__=='__main__':
 p=argparse.ArgumentParser();p.add_argument('config');a=p.parse_args();config=json.loads(Path(a.config).read_text());run(config)
