import json,os,subprocess,time
from pathlib import Path
root=Path('/home/alex/Projects/AG1');out=root/'implementation/evidence/overnight-fixrun-20260913'
env=os.environ.copy();env.update(CARGO_TARGET_DIR=str(root/'.local/p14-target'),CARGO_BUILD_JOBS='6',AG1_LLAMA_CPP='0');env['PATH']=str(root/'.local/toolchains/node-v24.14.0-linux-x64/bin')+':'+env['PATH']
base=['cargo','test','--manifest-path','product/Cargo.toml','--release','--locked']
rows=[]
checks=[('final-core',base+['-p','ag1-core','--test','context_contract','--test','context_history_contract','--test','context_continuation_contract','--test','runtime_context_prefix_contract','--test','task_guidance_contract']),
('final-storage',base+['-p','ag1-storage','--','--test-threads=1']),
('final-native',base+['-p','ag1-platform','--test','session_contract','--test','session_working_set_contract','--test','client_task_guidance_contract','--test','runtime_recovery_contract','--test','mcp_manager_contract','--test','mcp_execution_contract','--test','client_resume_contract','--','--test-threads=1']),
('final-desktop',base+['-p','ag1-desktop','--lib','--tests','--','--test-threads=1']),
('final-duration-tools',['python3','-m','unittest','discover','-s','tools/tests','-p','test_release_duration*.py']),
('final-renderer-build',['npm','--prefix','product/apps/desktop','run','build'])]
for label,cmd in checks:
 start=time.monotonic()
 with (out/(label+'.log')).open('w') as log:r=subprocess.run(cmd,cwd=root,env=env,stdout=log,stderr=subprocess.STDOUT)
 row={'label':label,'command':cmd,'exit_code':r.returncode,'seconds':time.monotonic()-start};rows.append(row);(out/'final-validation.json').write_text(json.dumps(rows,indent=2)+'\n');print(json.dumps(row),flush=True)
 if r.returncode:raise SystemExit(r.returncode)
with (out/'package-build.log').open('w') as log:
 subprocess.run(['python3',str(out/'prepare_candidate.py')],cwd=root,env=env,stdout=log,stderr=subprocess.STDOUT,check=True)
print('Final validation and packaging passed.',flush=True)
