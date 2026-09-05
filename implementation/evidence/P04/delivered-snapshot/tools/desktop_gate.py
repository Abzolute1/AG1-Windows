"""P04 execution gate. Native evidence is required; this gate never launches a desktop."""
import datetime,json,os,re,shutil,struct
from pathlib import Path
from common import ROOT,GateError,read_json,write_json,sha256,run,verify_original
STATES=['home','chat','files-dirty','diff','settings','dialog','loading','error']
def worker_result(text):
 counts={name:int(value) for name,value in re.findall(r'^[ℹ#]\s+(tests|pass|fail|cancelled|skipped|todo)\s+(\d+)\s*$',text,re.M)}
 if counts.get('tests',0)<19 or counts.get('pass')!=counts.get('tests') or any(counts.get(k,1) for k in ['fail','cancelled','skipped','todo']):raise GateError('Missing, failed or skipped worker regressions')
 return counts['pass']
def measurement(value,seconds=300):
 if value.get('duration_seconds',0)<seconds or len(value.get('samples',[]))<2:raise GateError('Incomplete native measurement')
 if not all(s.get('processes') and s.get('pss_kib',0)>0 for s in value['samples']):raise GateError('Missing process tree/PSS measurement')
 if value['samples'][-1]['time']-value['samples'][0]['time']<seconds:raise GateError('Measurement duration disagrees with samples')
 return value['samples'][-1]['pss_kib']/1024

def png(path):
 data=path.read_bytes()
 if len(data)<5000 or data[:8]!=b'\x89PNG\r\n\x1a\n' or struct.unpack('>II',data[16:24])!=(1440,900):raise GateError('Missing or unmatched native screenshot: '+str(path))
 return sha256(path)
def evidence_checks(evidence):
 retained={}
 for app in ['ag1','dyad']:
  visual=read_json(evidence/f'{app}-visual-states.json')
  if visual.get('states')!=STATES or visual.get('themes')!=['light','dark'] or visual.get('viewport')!={'width':1440,'height':900,'scale':1}:raise GateError('Incomplete native visual states: '+app)
  for state in STATES:
   hashes=[png(evidence/'screenshots'/f'{app}-{state}-{theme}.png') for theme in ['light','dark']]
   retained.update({f'{app}-{state}-{theme}.png':digest for theme,digest in zip(['light','dark'],hashes)})
   if hashes[0]==hashes[1]:raise GateError('Light and dark captures are identical')
  measurement(read_json(evidence/f'{app}-idle-metrics.json'))
  measurement(read_json(evidence/f'{app}-monaco-preview-metrics.json'),15)
  measurement(read_json(evidence/f'{app}-paused-renderer-metrics.json'),8)
 review=read_json(evidence/'visual-review.json')
 if review.get('status')!='reviewed' or review.get('screenshots')!=32:raise GateError('Native visual review is incomplete')
 if review.get('image_hashes')!=retained:raise GateError('Reviewed screenshots differ from retained native captures')
 native=read_json(evidence/'native-reconnect-preview.json')
 if not all(native.get(k) for k in ['pending_binding_restored','same_worker_after_reload','approved_once']):raise GateError('Native reconnect/approval failed')
 if not all(native['preview'].get(k) for k in ['opaqueOrigin','noParentDocument','focused','contained']) or native['preview']['messages']:raise GateError('Preview isolation failed')
 if not read_json(evidence/'native-paused-renderer.json').get('measurement_covers_pause'):raise GateError('Native pause was not measured')
 if not read_json(evidence/'native-large-approval.json').get('allow_disabled_until_final_page'):raise GateError('Full request review unavailable')
 for app in ['ag1','dyad']:
  if read_json(evidence/f'{app}-startup.json').get('startup_ms',0)<=0:raise GateError('Packaged startup missing')

def inputs():
 files=[]
 for base in [ROOT/'product',ROOT/'tools',ROOT/'fixtures/protocol']:
  files.extend(p for p in base.rglob('*') if p.is_file() and not any(part in ['node_modules','target','dist','__pycache__'] for part in p.relative_to(base).parts))
 return {str(p.relative_to(ROOT)):sha256(p) for p in sorted(set(files))}
def execute_p04():
 from phase_gate import verify_dependencies,verify_runtime_material,verify_artifacts
 evidence=ROOT/'implementation/evidence/P04';completion=evidence/'completion.json';write_json(completion,{'phase':'P04','status':'running'})
 commands=[]
 try:
  verify_dependencies('P04');original=verify_original();evidence_checks(evidence)
  node=ROOT/'.local/toolchains/node-v24.14.0-linux-x64/bin/node';env=os.environ.copy();env['PATH']=str(node.parent)+':'+env.get('PATH','/usr/bin:/bin');env['NO_COLOR']='1'
  npm=[str(node),str(node.parent/'../lib/node_modules/npm/bin/npm-cli.js')]
  tasks=[('bridge-reproduction',['python3','tools/build_bridge.py']),('runtime-preservation',['python3','tools/build_bridge_runtime.py']),('licenses',['python3','tools/record_protocol_licenses.py'])]
  for name,argv in tasks:commands.append(run(argv,log=evidence/f'{name}.log',env=env))
  before=inputs()
  tasks=[('gate-tests',['python3','-m','unittest','discover','-s','tools/tests','-p','test_desktop_gate.py','-v']),('worker-typecheck',npm+['--prefix','product/compat/agent-worker','run','typecheck']),('worker-tests',npm+['--prefix','product/compat/agent-worker','test']),('frontend-typecheck',npm+['--prefix','product/apps/desktop','run','typecheck']),('frontend-tests',npm+['--prefix','product/apps/desktop','test','--','--run']),('frontend-build',npm+['--prefix','product/apps/desktop','run','build']),('renderer-audit',[str(node),'tools/audit_desktop_bundle.mjs']),('format',['cargo','fmt','--manifest-path','product/Cargo.toml','-p','ag1-desktop','-p','ag1-protocol','--','--check']),('clippy',['cargo','clippy','--manifest-path','product/Cargo.toml','-p','ag1-desktop','--all-targets','--features','visual-test','--','-D','warnings']),('bridge-contract',['cargo','test','--manifest-path','product/Cargo.toml','-p','ag1-desktop','--test','bridge_contract']),('protocol-check',['cargo','run','--manifest-path','product/Cargo.toml','-p','ag1-protocol','--bin','protocol-codegen','--','--check']),('production-build',['cargo','build','--manifest-path','product/Cargo.toml','-p','ag1-desktop','-p','ag1-platform','--release']),('package',['python3','tools/package_bridge.py'])]
  for name,argv in tasks:
   print('P04: '+name,flush=True);commands.append(run(argv,log=evidence/f'{name}.log',env=env))
  count=worker_result((evidence/'worker-tests.log').read_text())
  if not re.search(r'Tests\s+6 passed \(6\)',(evidence/'frontend-tests.log').read_text()):raise GateError('Incomplete frontend regressions')
  if 'test result: ok. 3 passed; 0 failed; 0 ignored;' not in (evidence/'bridge-contract.log').read_text():raise GateError('Incomplete native contracts')
  audit=read_json(evidence/'renderer-bundle-audit.json')
  if audit['status']!='audited' or len(audit['files'])<1000 or len(audit['licenses'])<30:raise GateError('Incomplete renderer closure')
  verify_original();verify_dependencies('P04');verify_runtime_material(read_json(ROOT/'implementation/evidence/P01/runtime-manifest.json'),read_json(ROOT/'.local/runtime/hash-cache.json'))
  # The audit retains exact license files; they may be first generated during this gate.
  after=inputs()
  for path,digest in before.items():
   if after.get(path)!=digest:raise GateError('Input changed during gate: '+path)
  snapshot=evidence/'delivered-snapshot'
  for relative in after:
   target=snapshot/relative;target.parent.mkdir(parents=True,exist_ok=True);shutil.copyfile(ROOT/relative,target)
  artifacts=[{'path':str(p.relative_to(ROOT)),'sha256':sha256(p)} for p in sorted(evidence.rglob('*')) if p.is_file() and p!=completion and p.name!='gate.log']
  verify_artifacts(ROOT,artifacts)
  write_json(completion,{'phase':'P04','status':'passed','completed_at':datetime.datetime.now(datetime.timezone.utc).isoformat(),'dependencies':['P01','P02','P03'],'commands':commands,'input_fingerprints':after,'artifacts':artifacts,'source_verification':original,'coverage':{'worker_tests':count,'frontend_tests':6,'native_contracts':3,'screenshots':32,'invariants':['I01','I20','I21','I22','I23','I26','I30','L01','L02','L03','L04','L05','L06','L07','L08'],'defects':['B01','B02','B03','B04','D01']},'authority':'TS SDK owns executing agent/session/task/memory/artifact/model behavior; Rust owns native lifecycle, workspace services and separate preferences. One executing workspace. Intermediate bridge, not Rust parity.','pending_gates':read_json(evidence/'limitations.json'),'rollback':'Drain and stop only the owned worker; retain private state and exact logs. Use the immutable reference for supported export. Do not overwrite original config, profiles, runtimes or journals.'})
  print(json.dumps({'phase':'P04','status':'passed','worker_tests':count,'screenshots':32}))
 except Exception as exc:
  write_json(completion,{'phase':'P04','status':'failed','error':str(exc),'commands':commands});raise
