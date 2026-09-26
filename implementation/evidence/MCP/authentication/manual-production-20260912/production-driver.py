from pathlib import Path
import os,json,hashlib,subprocess,sys,time
root=Path('/home/alex/Projects/AG1');base=root/'.local/mcp-c1';node=root/'.local/toolchains/node-v24.14.0-linux-x64/bin/node'
sys.path.insert(0,str(root/'tools'));import release_gate,release_package
assert (base/'acceptance-ready.json').exists()
frozen=json.loads((base/'frozen-source.json').read_text());protected=json.loads((base/'preservation-start.json').read_text())
def sha(name):
 p=Path(name);p=p if p.is_absolute() else root/p
 return hashlib.file_digest(p.open('rb'),'sha256').hexdigest()
def verify():
 assert all(sha(n)==h for n,h in frozen.items())
 assert all(sha(n)==h for n,h in protected.items())
verify();commands={}
def run(name,args,cwd=root):
 commands[name]={'argv':[str(a) for a in args],'cwd':str(cwd),'started':time.time(),'state':'running'}
 (base/'production-commands.json').write_text(json.dumps(commands,indent=2)+'\n');print(name,'running',flush=True)
 with (base/(name+'.log')).open('x') as log:r=subprocess.run(args,cwd=cwd,stdout=log,stderr=subprocess.STDOUT)
 commands[name].update(exit_code=r.returncode,state='finished',finished=time.time());(base/'production-commands.json').write_text(json.dumps(commands,indent=2)+'\n');print(name,r.returncode,flush=True);assert r.returncode==0,name
run('production-typecheck',[node,'node_modules/typescript/bin/tsc','--noEmit'],root/'product/apps/desktop')
run('production-renderer',[node,'node_modules/vite/bin/vite.js','build'],root/'product/apps/desktop')
inputs=release_gate.fingerprints();(base/'production-inputs.json').write_text(json.dumps(inputs,indent=2)+'\n')
run('production-package',['python3',base/'package.py'])
report=json.loads((base/'package.json').read_text());assert report['input_sha256']==release_gate.input_digest();verify()
release=release_package.validate_candidate(report,report['input_sha256'],root,base/'validated-package')
(base/'package-validation.json').write_text(json.dumps({'status':'passed','release':str(release),'archive':report['archive'],'input_sha256':report['input_sha256']},indent=2)+'\n')
for name in ['ag1-cli','ag1-cutover']:run(name+'-production-help',[release/'bin'/name,'--help'])
run('production-smoke',['python3',base/'production-smoke.py'])
verify();assert release_gate.input_digest()==report['input_sha256']
(base/'production-input-verification.json').write_text(json.dumps({'status':'passed','frozen_source_files':len(frozen),'protected_files':len(protected),'input_sha256':report['input_sha256'],'unchanged':True},indent=2)+'\n')
print('Production verification finished; inspect screenshots before per-user installation',flush=True)
