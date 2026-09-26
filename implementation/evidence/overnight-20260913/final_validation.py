"""Run the final frontend and Rust milestone after selective follow-up merge."""
import hashlib,json,os,re,subprocess,sys,time
from pathlib import Path
ROOT=Path('/home/alex/Projects/AG1');OUT=ROOT/'implementation/evidence/overnight-20260913'
label=sys.argv[1] if len(sys.argv)>1 else 'final'
if not re.fullmatch(r'[a-z0-9-]+',label):raise RuntimeError('Invalid evidence label')
merge=json.loads((OUT/'followup-merge-result.json').read_text())
expected={row['path']:row['candidate_sha256'] for row in merge['files']}
additional=OUT/'post-run7-additional-changes.json'
if additional.exists():
 for row in json.loads(additional.read_text())['files']:expected[row['path']]=row['sha256']
for path,sha in expected.items():
 if hashlib.sha256((ROOT/path).read_bytes()).hexdigest()!=sha:raise RuntimeError('Declared candidate changed before final validation: '+path)
env=os.environ.copy();env['PATH']=str(ROOT/'.local/toolchains/node-v24.14.0-linux-x64/bin')+':'+env['PATH'];env['CARGO_TARGET_DIR']=str(ROOT/'.local/p14-target');env['CARGO_BUILD_JOBS']='6'
cases=[('desktop-tests',['npm','exec','--','vitest','run'],ROOT/'product/apps/desktop'),('desktop-build',['npm','run','build'],ROOT/'product/apps/desktop'),('rust-workspace',['cargo','test','--manifest-path','product/Cargo.toml','--offline','--locked','--release','--workspace','--no-fail-fast'],ROOT)]
rows=[]
for name,command,cwd in cases:
 log=OUT/f'{label}-{name}.log';started=time.monotonic()
 with log.open('x') as stream:run=subprocess.run(command,cwd=cwd,env=env,stdout=stream,stderr=subprocess.STDOUT)
 row={'case':name,'command':command,'cwd':str(cwd),'exit_code':run.returncode,'seconds':time.monotonic()-started,'log':str(log)};rows.append(row);print(json.dumps(row),flush=True)
 (OUT/f'{label}-validation-results.json').write_text(json.dumps(rows,indent=2)+'\n')
 if run.returncode:raise SystemExit(run.returncode)
