from pathlib import Path
import os,subprocess,json,hashlib,shutil,time
root=Path('/home/alex/Projects/AG1');out=root/'.local/mcp-c1';target=root/'.local/p14-target'
for profile in ['mcp-c1-final','mcp-c1-before']:
 assert not (target/profile).exists(),f'Fresh profile required: {profile}'
env=dict(os.environ,CARGO_TARGET_DIR=str(target),CARGO_INCREMENTAL='0')
node=str(root/'.local/toolchains/node-v24.14.0-linux-x64/bin/node')
commands={}
def sha(path):return hashlib.sha256(Path(path).read_bytes()).hexdigest()
def save(path,value):path.write_text(json.dumps(value,indent=2)+'\n')
def run(name,args,cwd=root,allowed=(0,),extra=None):
 commands[name]={'argv':args,'cwd':str(cwd),'started':time.time(),'state':'running'}
 save(out/'final-commands.json',commands)
 print(name,'running',flush=True)
 with (out/(name+'.log')).open('w') as log:
  r=subprocess.run(args,cwd=cwd,env=dict(env,**(extra or {})),stdout=log,stderr=subprocess.STDOUT)
 commands[name].update(exit_code=r.returncode,finished=time.time(),state='finished')
 save(out/'final-commands.json',commands)
 print(name,r.returncode,flush=True)
 assert r.returncode in allowed,(name,r.returncode)
def cargo(profile,action):return ['cargo',action,'--offline','--locked','--manifest-path','product/Cargo.toml','--config',f'profile.{profile}.inherits="dev"','--config',f'profile.{profile}.debug=0','--profile',profile]
paths=set(json.loads((out/'before-sha256.json').read_text()))
for directory in ['product/crates/ag1-core','product/crates/ag1-platform','product/crates/ag1-storage','product/crates/ag1-protocol','product/apps/desktop/src','product/apps/desktop/src-tauri/src']:
 paths.update(str(p.relative_to(root)) for p in (root/directory).rglob('*') if p.is_file() and p.suffix in ['.rs','.py','.json','.txt','.sql','.toml','.ts','.tsx','.css'])
paths.update(['tools/probes/mcp_native_private_audit.mjs','tools/probes/mcp_private_input.py','tools/probes/mcp_native_manager_audit.mjs','tools/probes/p14_native_startup_smoke.mjs','product/apps/desktop/vite.config.ts','product/apps/desktop/package.json','product/apps/desktop/package-lock.json','product/apps/desktop/src-tauri/Cargo.toml','product/apps/desktop/src-tauri/tauri.conf.json'])
frozen={p:sha(root/p) for p in sorted(paths)};save(out/'frozen-source.json',frozen)
protected=json.loads((out/'preservation-start.json').read_text())
preservation={p:{'expected':h,'actual':sha(root/p)} for p,h in protected.items()}
assert all(v['expected']==v['actual'] for v in preservation.values())
save(out/'final-preservation-start.json',preservation)
run('final-contracts',cargo('mcp-c1-final','test')+['-p','ag1-core','--test','mcp_registry_contract','--test','mcp_capability_contract','--test','mcp_discovery_contract','--test','mcp_owner_contract','-p','ag1-platform','--test','mcp_auth_contract','--test','mcp_execution_contract','--test','mcp_manager_contract','--test','mcp_http_contract','--test','mcp_stdio_contract','--test','mcp_foundation_contract','--test','platform_contract','--test','github_local_contract','--test','client_folder_access_contract','-p','ag1-storage','--test','mcp_private_contract','--test','mcp_storage_contract','--test','storage_contract','--test','tool_storage_contract','--','--test-threads=1'])
run('final-units',cargo('mcp-c1-final','test')+['-p','ag1-platform','--lib','mcp::','--','--test-threads=1'])
run('final-clippy',cargo('mcp-c1-final','clippy')+['-p','ag1-platform','--lib'])
run('final-session',cargo('mcp-c1-final','test')+['-p','ag1-platform','--test','session_contract','--','--test-threads=1'],allowed=(0,101))
run('final-core',cargo('mcp-c1-final','test')+['-p','ag1-core','--test','tool_contract','--','--test-threads=1'],allowed=(0,101))
run('baseline-session',cargo('mcp-c1-before','test')+['-p','ag1-platform','--test','session_contract','--','--test-threads=1'],cwd=out/'baseline',allowed=(0,101))
run('baseline-core',cargo('mcp-c1-before','test')+['-p','ag1-core','--test','tool_contract','--','--test-threads=1'],cwd=out/'baseline',allowed=(0,101))
desktop=root/'product/apps/desktop'
run('final-typecheck',[node,'node_modules/typescript/bin/tsc','--noEmit'],cwd=desktop)
run('final-ui',[node,'node_modules/vitest/vitest.mjs','run'],cwd=desktop,allowed=(0,1))
run('baseline-ui',[node,'node_modules/vitest/vitest.mjs','run'],cwd=out/'baseline/product/apps/desktop',allowed=(0,1))
candidate=root/'.local/p14-live-fixes/native-candidate-mcp-c1-final'
assert not candidate.exists();candidate.mkdir()
run('final-renderer',[node,'node_modules/vite/bin/vite.js','build','--outDir',str(candidate/'renderer')],cwd=desktop)
run('final-desktop',cargo('mcp-c1-final','build')+['-p','ag1-desktop','--features','visual-test'],extra={'TAURI_CONFIG':json.dumps({'build':{'frontendDist':str(candidate/'renderer')}})})
for name in ['ag1-desktop','ag1-process-host']:shutil.copy2(target/'mcp-c1-final'/name,candidate/name)
run('final-native-ui',[node,'tools/probes/mcp_native_manager_audit.mjs',str(candidate)])
run('final-private-native-ui',[node,'tools/probes/mcp_native_private_audit.mjs',str(candidate)])
assert all(sha(root/p)==h for p,h in frozen.items()),'Source changed during final checks'
assert all(sha(root/p)==h for p,h in protected.items()),'Protected input changed during checks'
save(out/'final-source-verification.json',{'source_count':len(frozen),'protected_count':len(protected),'unchanged':True,'completed':time.time()})
print('CHECKS FINISHED; comparison, screenshots, evidence acceptance and production installation still required',flush=True)
