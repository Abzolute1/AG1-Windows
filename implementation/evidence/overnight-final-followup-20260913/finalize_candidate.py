import json,os,subprocess,sys,time
from pathlib import Path
ROOT=Path('/home/alex/Projects/AG1');OUT=ROOT/'implementation/evidence/overnight-final-followup-20260913'
os.chdir(ROOT);sys.path.insert(0,str(ROOT/'tools'))
from release_package import package,validate_candidate
from release_gate import fingerprints,ref
before=fingerprints();env=os.environ.copy();env.update(CARGO_TARGET_DIR=str(ROOT/'.local/p14-target'),CARGO_BUILD_JOBS='6',AG1_LLAMA_CPP='0')
command=['cargo','test','--manifest-path','product/Cargo.toml','--release','--locked','-p','ag1-platform','--test','session_contract','--test','session_working_set_contract','--test','client_task_guidance_contract','--','--test-threads=1']
start=time.monotonic()
with (OUT/'final-native.log').open('w') as log:result=subprocess.run(command,env=env,stdout=log,stderr=subprocess.STDOUT)
(OUT/'final-validation.json').write_text(json.dumps({'command':command,'exit_code':result.returncode,'seconds':time.monotonic()-start},indent=2)+'\n')
if result.returncode:raise SystemExit(result.returncode)
print('Final native checks passed.',flush=True)
report=package();(OUT/'package.json').write_text(json.dumps(report,indent=2)+'\n')
installed=validate_candidate(report,report['input_sha256'],ROOT,ROOT/'.local/overnight-final-followup-install-check-v3')
(OUT/'package-validation.json').write_text(json.dumps({'status':'passed','scope':'Isolated checksum-verifying installer','manifest':ref(installed/'package-manifest.json')},indent=2)+'\n')
if fingerprints()!=before:raise RuntimeError('Source inputs changed during final build')
print('Final candidate packaged and validated.',flush=True)
