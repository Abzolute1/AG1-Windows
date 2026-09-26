"""Validate isolated followups against the refreshed frozen-candidate baseline."""
import json,os,subprocess,time
from pathlib import Path
root=Path('/home/alex/Projects/AG1/implementation/evidence/overnight-20260913')
base=['cargo','test','--manifest-path','product/Cargo.toml','--offline','--locked','--release']
cases=[
 ('storage', ['-p','ag1-storage']),
 ('core', ['-p','ag1-core','--test','tool_contract','--test','context_history_contract','--test','context_contract']),
 ('projection', ['-p','ag1-platform','--lib','client::projection']),
 ('client', ['-p','ag1-platform','--test','client_conversation_contract','--test','client_receipt_contract']),
 ('cli', ['-p','ag1-cli','--test','client_contract','packaged_jsonl_stdout_is_framed_correlated_and_eof_drains_native_work']),
]
env=os.environ.copy();env['CARGO_TARGET_DIR']='/tmp/ag1-github-release-target';env['PATH']='/home/alex/Projects/AG1/.local/toolchains/node-v24.14.0-linux-x64/bin:'+env['PATH']
results=[]
for name,args in cases:
 path=root/f'performance-combined-after-{name}.log';start=time.monotonic()
 with path.open('x') as log:run=subprocess.run(base+args,cwd='/tmp/ag1-overnight-followup',env=env,stdout=log,stderr=subprocess.STDOUT)
 row={'case':name,'command':base+args,'exit_code':run.returncode,'seconds':time.monotonic()-start,'log':str(path)};results.append(row);print(json.dumps(row),flush=True)
 (root/'performance-combined-after-results.json').write_text(json.dumps(results,indent=2))
 if run.returncode:raise SystemExit(run.returncode)
