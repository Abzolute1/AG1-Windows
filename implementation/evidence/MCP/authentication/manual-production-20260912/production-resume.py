from pathlib import Path
import sys,subprocess,json,hashlib,time
root=Path('/home/alex/Projects/AG1');base=root/'.local/mcp-c1';sys.path.insert(0,str(root/'tools'));import release_gate
report=json.loads((base/'package.json').read_text());assert report['input_sha256']==release_gate.input_digest()
assert json.loads((base/'package-validation.json').read_text())['status']=='passed'
old=root/'.local/mcp-b6/validated-package/current/bin/ag1-cutover'
run=subprocess.run([old,'--help'],stdout=subprocess.PIPE,stderr=subprocess.STDOUT);(base/'baseline-cutover-usage.log').write_bytes(run.stdout)
current=(base/'ag1-cutover-production-help.log').read_bytes();assert run.returncode==1 and run.stdout==current
assert current.startswith(b'ag1-cutover: Usage:')
review={'status':'verified_existing_behavior','baseline_exit':run.returncode,'candidate_exit':1,'identical_usage_output':True,'reason':'ag1-cutover has no --help success branch; unknown/missing command prints usage and exits 1 in preserved baseline and candidate. Original driver incorrectly expected zero; its failure log is retained.','baseline_binary':str(old.resolve())}
(base/'cutover-usage-review.json').write_text(json.dumps(review,indent=2)+'\n')
print('Preserved baseline and candidate cutover usage agree; continuing existing production package',flush=True)
with (base/'production-smoke.log').open('x') as log:r=subprocess.run(['python3',base/'production-smoke.py'],cwd=root,stdout=log,stderr=subprocess.STDOUT)
assert r.returncode==0,'production smoke failed; inspect retained logs'
frozen=json.loads((base/'frozen-source.json').read_text());protected=json.loads((base/'preservation-start.json').read_text())
def sha(n):
 p=Path(n);p=p if p.is_absolute() else root/p
 return hashlib.file_digest(p.open('rb'),'sha256').hexdigest()
assert all(sha(n)==h for n,h in frozen.items());assert all(sha(n)==h for n,h in protected.items());assert release_gate.input_digest()==report['input_sha256']
(base/'production-input-verification.json').write_text(json.dumps({'status':'passed','frozen_source_files':len(frozen),'protected_files':len(protected),'input_sha256':report['input_sha256'],'unchanged':True,'cutover_usage_review':'cutover-usage-review.json','production_smoke_exit':r.returncode},indent=2)+'\n')
print('Production startup and preservation passed; screenshots require review',flush=True)
