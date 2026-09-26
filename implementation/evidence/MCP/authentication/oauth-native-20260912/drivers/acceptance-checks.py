from pathlib import Path
import os,subprocess,json,hashlib,shutil,time
root=Path('/home/alex/Projects/AG1');out=root/'.local/mcp-c2';target=root/'.local/p14-target'
env=dict(os.environ,CARGO_TARGET_DIR=str(target),CARGO_INCREMENTAL='0')
node=str(root/'.local/toolchains/node-v24.14.0-linux-x64/bin/node')
commands={}
def sha(path):return hashlib.sha256(Path(path).read_bytes()).hexdigest()
def save(path,value):path.write_text(json.dumps(value,indent=2)+'\n')
def run(name,args,cwd=root,allowed=(0,),extra=None):
 commands[name]={'argv':args,'cwd':str(cwd),'started':time.time(),'state':'running'}
 save(out/'acceptance-commands.json',commands)
 print(name,'running',flush=True)
 with (out/(name+'.log')).open('w') as log:
  r=subprocess.run(args,cwd=cwd,env=dict(env,**(extra or {})),stdout=log,stderr=subprocess.STDOUT)
 commands[name].update(exit_code=r.returncode,finished=time.time(),state='finished')
 save(out/'acceptance-commands.json',commands)
 print(name,r.returncode,flush=True)
 assert r.returncode in allowed,(name,r.returncode)
def cargo(profile,action):return ['cargo',action,'--offline','--locked','--manifest-path','product/Cargo.toml','--config',f'profile.{profile}.inherits="dev"','--config',f'profile.{profile}.debug=0','--profile',profile]
frozen=json.loads((out/'ui3-source.json').read_text())
assert all(sha(root/p)==h for p,h in frozen.items())
run('acceptance-contracts',cargo('mcp-c2','test')+['-p','ag1-core','--test','mcp_registry_contract','--test','mcp_capability_contract','--test','mcp_discovery_contract','--test','mcp_owner_contract','-p','ag1-platform','--test','mcp_oauth_contract','--test','mcp_auth_contract','--test','mcp_execution_contract','--test','mcp_manager_contract','--test','mcp_http_contract','--test','mcp_stdio_contract','--test','mcp_foundation_contract','--test','platform_contract','--test','github_local_contract','--test','client_folder_access_contract','-p','ag1-storage','--test','mcp_private_contract','--test','mcp_storage_contract','--test','storage_contract','--test','tool_storage_contract','--','--test-threads=1'])
run('acceptance-units',cargo('mcp-c2','test')+['-p','ag1-platform','--lib','mcp::','--','--test-threads=1'])
run('acceptance-clippy',cargo('mcp-c2','clippy')+['-p','ag1-platform','--lib'])
run('acceptance-session',cargo('mcp-c2','test')+['-p','ag1-platform','--test','session_contract','--','--test-threads=1'],allowed=(0,101))
run('acceptance-core',cargo('mcp-c2','test')+['-p','ag1-core','--test','tool_contract','--','--test-threads=1'],allowed=(0,101))

desktop=root/'product/apps/desktop'
run('acceptance-typecheck',[node,'node_modules/typescript/bin/tsc','--noEmit'],cwd=desktop)
run('acceptance-ui',[node,'node_modules/vitest/vitest.mjs','run'],cwd=desktop,allowed=(0,1))
assert all(sha(root/p)==h for p,h in frozen.items())
print('REGRESSION COMMANDS FINISHED; inspect comparisons before acceptance',flush=True)
