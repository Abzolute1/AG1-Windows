import copy,hashlib,json,os,subprocess
from pathlib import Path
ROOT=Path('/home/alex/Projects/AG1');R=ROOT/'.local/ag1-optimized-validation-20260914';E=ROOT/'implementation/evidence/ag1-optimized-validation-20260914';P=ROOT/'implementation/evidence/llama-performance-20260914'
def sha(p):
 with Path(p).open('rb')as f:return hashlib.file_digest(f,'sha256').hexdigest()
def save(p,x):
 p.write_text(json.dumps(x,indent=2)+'\n');p.chmod(0o600);return {'path':str(p),'sha256':sha(p)}
source=json.loads(Path('/home/alex/.local/state/ag1/desktop/configuration/runtime-configuration-vision-20260909.json').read_text());base=source['request'];model=next(x for x in base['models'] if '35B' in x['model_id']);mid=model['model_id']
profiles=json.loads(Path(base['profile']['path']).read_text());cfgbase=json.loads((ROOT/'.local/chat-launcher-recovery-20260914/probe-config.json').read_text())
bundle=Path('/home/alex/Projects/llama-perf-20260914/selected-rows32-bundle/bundle-v1');selected=json.loads((bundle/'recommended-launch.json').read_text());bm=json.loads((bundle/'bundle-manifest.json').read_text())
protected=json.loads((P/'protected-after.json').read_text());save(E/'protected-before.json',protected)
for arm in ['original','candidate']:
 d=R/arm;d.mkdir(mode=0o700);profile=copy.deepcopy(profiles['models'][mid]);m=copy.deepcopy(model)
 env=json.loads(Path(base['inherited_environment']['path']).read_text());env['LLAMA_CPP_PORT']='18436'
 args=m['expected']['args'];args[args.index('--port')+1]='18436'
 originalroot=Path('/home/alex/Projects/llama-cpp-turboquant/build/bin')
 if arm=='candidate':
  profile.update(serverPath=str(bundle/'bin/llama-server'),libPath=str(bundle/'bin'),batchSize=4096,ubatchSize=4096,decodeThreads=6,batchThreads=12,nCpuMoe=32,loadMode='mmap',environment=selected['environment'])
  profile['extraArgs'] += ['--moe-cache','4480','--spec-type','ngram-mod','--spec-ngram-mod-n-max','16','--spec-ngram-mod-n-min','2']
  for key,value in {'--batch-size':4096,'--ubatch-size':4096,'--threads':6,'--threads-batch':12,'--n-cpu-moe':32,'--load-mode':'mmap','--cpu-range':'0-5'}.items():args[args.index(key)+1]=str(value)
  args += profile['extraArgs'][1:]
  m['materials']=[{'path':str(bundle/'bin'/x['destination']),'sha256':x['sha256'],'kind':'executable' if x['destination']=='llama-server' else 'library'}for x in bm['files']]
 else:
  m['materials']=[x for x in model['materials'] if Path(x['path']).is_relative_to(originalroot) or x['path']=='/etc/ld.so.cache']
 for x in model['materials']:
  if x['path'] in [model['model_path'],model['discovered_projector']]:m['materials'].append(x)
 # No inherited experimental backend overrides; match selected profile's runtime environment.
 if arm=='candidate':
  for k in list(env):
   if k.startswith(('GGML_','LLAMA_ARG_','LLAMA_MOE_','LLAMA_MMAP_','GOMP_','OMP_')) or k in ['LD_PRELOAD','LD_LIBRARY_PATH']:env.pop(k)
 expectedenv={**env,**profile.get('environment',{})};lib=profile['libPath'];expectedenv['LD_LIBRARY_PATH']=lib+(':'+expectedenv['LD_LIBRARY_PATH'] if expectedenv.get('LD_LIBRARY_PATH') else '')
 m['expected'].update(command=profile['serverPath'],args=args,environment=expectedenv)
 req={**copy.deepcopy(base),'models':[m],'profile':save(d/'profiles.json',{'version':1,'models':{mid:profile}}),'inherited_environment':save(d/'environment.json',env),'additional_inputs':{}}
 save(d/'import-request.json',req)
 command=['/opt/ag1/current/bin/ag1-cutover','capture-runtime',str(d/'import-request.json'),str(d/'capture.json')]
 with (d/'capture.stdout').open('w')as out,(d/'capture.stderr').open('w')as err:
  result=subprocess.run(command,stdout=out,stderr=err,timeout=300)
 if result.returncode:raise RuntimeError('Capture failed for '+arm+'; inspect private stderr')
 receipt=json.loads((d/'capture.stdout').read_text());save(d/'capture-receipt.json',receipt)
 cfg=copy.deepcopy(cfgbase);cfg['models']=[next(x for x in cfg['models'] if x['id']==mid)];cfg['models'][0]['baseUrl']='http://127.0.0.1:18436/v1';cfg['credentials']={};cfg['credential_env']=None;cfg['extensions']=None;cfg['packages']=None
 settings=json.loads(Path(cfg['imported']['settings']['path']).read_text());cfg['imported']={'settings':save(d/'settings.json',settings),'credentials':None}
 cfg['runtime']['capture']=receipt;cfg['runtime']['exclusive_gpu']=False;cfg['max_generations']=12;cfg['policy_mode']='autonomous';save(d/'config.json',cfg)
 save(E/(arm+'-launch.json'),{'binary':profile['serverPath'],'args':args,'selected_environment':profile.get('environment',{}),'model':mid,'configuration_capture_sha256':receipt['sha256'],'scope':'Private one-model capture; no production configuration changed. Candidate CPU range resolves to 0-5 from six decode threads; this range flag is ineffective in the tested OpenMP server path.'})
 print('Captured',arm,flush=True)
save(E/'setup.json',{'ag1_release':str(Path('/opt/ag1/current').resolve()),'ag1_cli_sha256':sha('/opt/ag1/current/bin/ag1-cli'),'model':mid,'private_root':str(R),'port':18436,'screen_capture':False,'scope':'Installed native AG1 CLI/Host/session/runtime/tool paths; not renderer visual testing.'})
