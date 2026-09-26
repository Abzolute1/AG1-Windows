import json, pathlib, subprocess, os, signal, time, hashlib, shutil, sys
root=pathlib.Path('/home/alex/Projects/AG1');scratch=root/'.local/search-quality-20260914'; evidence=root/'implementation/evidence/search-quality-20260914'
label=sys.argv[1];binary=pathlib.Path(sys.argv[2]).resolve();run=scratch/label;run.mkdir()
cfg=json.loads((root/'.local/p14-long-horizon/scenario-validation-20260913/candidate/config.json').read_text())
cfg['max_generations']=18
cfg['policy_mode']='autonomous'
cfg['system_prompt']='You are a research assistant. Use the available search and page-reading tools and report evidence accurately.'
# Isolated import copies preserve the user's source settings and credentials.
for name,spec in cfg['imported'].items():
 if isinstance(spec,dict) and 'path' in spec:
  source=pathlib.Path(spec['path']); target=run/source.name;shutil.copyfile(source,target);os.chmod(target,0o600);spec['path']=str(target)
  if 'sha256' in spec:spec['sha256']=hashlib.sha256(target.read_bytes()).hexdigest()
config=run/'config.json';config.write_text(json.dumps(cfg));os.chmod(config,0o600)
workspace=run/'workspace';subprocess.run([str(binary),'--init',str(workspace)],check=True,capture_output=True)
prompt='Research Python HTTP retry behavior using current official documentation. Make two distinct webSearch queries and fetch at least two relevant official pages, including urllib3 Retry documentation and MDN Retry-After documentation. Use at most three searches and three page fetches. Answer in under 450 words: What is urllib3 Retry\'s default allowed_methods set, is POST included, and why is retrying a timed-out order-creation POST unsafe? What are both Retry-After forms and how should a past date be handled? Give short pseudocode for a 12-second overall retry deadline using a monotonic clock, distinguishing your policy from library defaults. Cite sources beside the claims. If a tool fails, report it accurately and continue with available evidence. Do not create or modify files.'
(evidence/'model-prompt.txt').write_text(prompt+'\n')
cmd=[str(binary),'--workspace',str(workspace),'--state',str(run/'state'),'--config',str(config),'--mode','jsonl','--tools','webSearch,webFetch','--',prompt]
started=time.monotonic();timedout=False
with (evidence/(label+'-stdout.txt')).open('w') as out,(evidence/(label+'-stderr.txt')).open('w') as err:
 p=subprocess.Popen(cmd,stdout=out,stderr=err,stdin=subprocess.DEVNULL,start_new_session=True,cwd=root)
 try:p.wait(timeout=240)
 except subprocess.TimeoutExpired:
  timedout=True;os.killpg(p.pid,signal.SIGTERM)
  try:p.wait(timeout=10)
  except subprocess.TimeoutExpired:os.killpg(p.pid,signal.SIGKILL);p.wait()
report={'binary':str(binary),'binarySha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'exitCode':p.returncode,'timedOut':timedout,'elapsedSeconds':time.monotonic()-started,'run':str(run),'model':cfg['selected'],'maxGenerations':18}
(evidence/(label+'-run.json')).write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report))
