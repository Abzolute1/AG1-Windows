import os,json,subprocess,time,resource
from pathlib import Path
r=Path(__file__).parent;e=Path('/home/alex/Projects/AG1/implementation/evidence/llama-performance-20260914');resource.setrlimit(resource.RLIMIT_CORE,(0,0))
while True:
 live=False
 for proc in Path('/proc').iterdir():
  if not proc.name.isdigit():continue
  try:args=(proc/'cmdline').read_bytes().split(b'\0')
  except OSError:continue
  if str(r/'phase9.py').encode() in args:live=True;break
 if not live:break
 time.sleep(2)
last=json.loads((e/'runs/memory-original-control-b/result.json').read_text());assert last['status']=='passed'
steps=[]
def stage(name,cmd,env=None,timeout=1200):
 start=time.monotonic()
 with (e/(name+'.log')).open('w') as log:
  try:rc=subprocess.run(cmd,stdout=log,stderr=subprocess.STDOUT,env=env,timeout=timeout).returncode
  except subprocess.TimeoutExpired:rc=124
 row={'name':name,'exit_code':rc,'seconds':time.monotonic()-start};steps.append(row);(e/'setup10.json').write_text(json.dumps(steps,indent=2));print(json.dumps(row),flush=True);return rc==0
built=stage('cpu-matched-build',['python',str(r/'cpu-mmid-single/build-matched.py')])
stage('mmap-v7-build',['bash',str(r/'build-mmap-precise-v7.sh')])
if built:
 task=r/'cpu-mmid-single';diag=task/'runtime-diagnostic'
 if stage('cpu-fixture-build',['bash',str(task/'build-fixture.sh'),str(diag)]):
  env=os.environ.copy();env.pop('LD_PRELOAD',None);env['LD_LIBRARY_PATH']=str(diag)
  stage('cpu-fixture-diagnostic',[str(task/'test-mmid-single')],env,600)
  for variant in ['control','patched']:
   env['LD_LIBRARY_PATH']=str(task/('runtime-'+variant))
   stage('cpu-fixture-'+variant,[str(task/'test-mmid-single'),'--allow-no-coverage'],env,600)
  for flag in ['0','1']:
   env['LD_LIBRARY_PATH']=str(task/'runtime-patched');env['GGML_CPU_MOE_SINGLE_TOKEN']=flag
   stage('cpu-mmid-backend-'+flag,[str(task/'runtime-patched/test-backend-ops'),'test','-b','CPU','-o','MUL_MAT_ID','-j','1'],env,1200)
stage('draft-model-download',['python',str(r/'download-draft.py')],timeout=600)
# Configuration-only timings proceed independently of whether a source fixture failed.
with (e/'phase10.log').open('w') as log:
 result=subprocess.run(['python',str(r/'phase10.py')],stdout=log,stderr=subprocess.STDOUT)
print('phase10_exit',result.returncode,flush=True)
