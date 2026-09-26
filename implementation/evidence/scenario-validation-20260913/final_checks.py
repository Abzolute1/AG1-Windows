import os,json,subprocess,time
from pathlib import Path
r=Path.cwd();e=r/'implementation/evidence/scenario-validation-20260913';env=dict(os.environ,CARGO_TARGET_DIR=str(r/'.local/p14-target'),CARGO_BUILD_JOBS='6',AG1_LLAMA_CPP='0');env['PATH']=str(r/'.local/toolchains/node-v24.14.0-linux-x64/bin')+':'+env['PATH']
base=['cargo','test','--manifest-path','product/Cargo.toml','--release','--locked']
jobs=[('native',base+['-p','ag1-platform','--test','session_contract','--test','session_working_set_contract','--test','client_task_guidance_contract','--','--test-threads=1']),('work-surface',base+['-p','ag1-core','--test','native_surface_contract','--','--test-threads=1']),('custody',base+['-p','ag1-storage','--lib','blob_codec::tests','--','--test-threads=1']),('mcp-helper',base+['-p','ag1-desktop','--lib','packaged_helper_tests','--','--test-threads=1'])]
rows=[]
for name,cmd in jobs:
 t=time.monotonic()
 with (e/f'final-{name}.log').open('x') as log:p=subprocess.run(cmd,env=env,stdout=log,stderr=subprocess.STDOUT)
 row={'name':name,'command':cmd,'exit_code':p.returncode,'seconds':time.monotonic()-t};rows.append(row);(e/f'final-{name}.json').write_text(json.dumps(row,indent=2)+'\n');print(json.dumps(row),flush=True)
(e/'final-checks.json').write_text(json.dumps(rows,indent=2)+'\n')
raise SystemExit(any(row['exit_code'] for row in rows))
