from pathlib import Path
import os,subprocess,json,hashlib
root=Path('/home/alex/Projects/AG1');out=root/'.local/mcp-b5'
env=dict(os.environ,CARGO_TARGET_DIR=str(root/'.local/p14-target'),CARGO_INCREMENTAL='0')
commands={}
def run(name,args,cwd=root,allowed=(0,)):
 commands[name]={'argv':args,'cwd':str(cwd)}
 with (out/(name+'.log')).open('w') as log:
  r=subprocess.run(args,cwd=cwd,env=env,stdout=log,stderr=subprocess.STDOUT)
 commands[name]['exit_code']=r.returncode
 (out/'final-commands.json').write_text(json.dumps(commands,indent=2)+'\n')
 print(name,r.returncode,flush=True)
 assert r.returncode in allowed,(name,r.returncode)
def cargo(profile,action):return ['cargo',action,'--offline','--locked','--manifest-path','product/Cargo.toml','--config',f'profile.{profile}.inherits="dev"','--config',f'profile.{profile}.debug=0','--profile',profile]
paths=[]
for crate in ['ag1-core','ag1-platform','ag1-storage','ag1-protocol']:
 paths.extend(p for p in (root/'product/crates'/crate).rglob('*') if p.is_file() and p.suffix in ['.rs','.py','.json','.txt','.sql','.toml'])
(out/'frozen-source.json').write_text(json.dumps({str(p.relative_to(root)):hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted(paths)},indent=2)+'\n')
run('final-format',['rustfmt','--check','--edition','2024','--config','skip_children=true',*['product/'+p for p in ['crates/ag1-core/src/tools/mcp/discovery.rs','crates/ag1-core/tests/mcp_discovery_contract.rs','crates/ag1-platform/src/session/mcp.rs','crates/ag1-platform/src/session/mcp/discovery.rs','crates/ag1-platform/tests/mcp_runtime/discovery.rs']]])
run('final-contracts',cargo('mcp-b5-final','test')+['-p','ag1-core','--test','mcp_registry_contract','--test','mcp_capability_contract','--test','mcp_discovery_contract','-p','ag1-platform','--test','mcp_execution_contract','--test','mcp_manager_contract','--test','mcp_http_contract','--test','mcp_stdio_contract','--test','mcp_foundation_contract','--test','platform_contract','--test','github_local_contract','-p','ag1-storage','--test','mcp_storage_contract','--test','storage_contract','--test','tool_storage_contract','--','--test-threads=1'])
run('final-units',cargo('mcp-b5-final','test')+['-p','ag1-platform','--lib','mcp::','--','--test-threads=1'])
run('final-clippy',cargo('mcp-b5-final','clippy')+['-p','ag1-platform','--lib'])
run('final-session',cargo('mcp-b5-final','test')+['-p','ag1-platform','--test','session_contract','--','--test-threads=1'],allowed=(0,101))
run('final-core',cargo('mcp-b5-final','test')+['-p','ag1-core','--test','tool_contract','--','--test-threads=1'],allowed=(0,101))
run('baseline-session',cargo('mcp-b5-before','test')+['-p','ag1-platform','--test','session_contract','--','--test-threads=1'],cwd=out/'baseline',allowed=(0,101))
run('baseline-core',cargo('mcp-b5-before','test')+['-p','ag1-core','--test','tool_contract','--','--test-threads=1'],cwd=out/'baseline',allowed=(0,101))
