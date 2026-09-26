from pathlib import Path
import os,subprocess,time,json,hashlib
root=Path('/home/alex/Projects/AG1');out=root/'.local/mcp-b4'
# These two earlier owned runs shared a misdirected log. Preserve it as diagnostic
# only, wait for their natural fixture cleanup, and never accept it as evidence.
end=time.monotonic()+240
for pid in [171505,173153]:
 while True:
  try:os.kill(pid,0)
  except ProcessLookupError:break
  assert time.monotonic()<end,'prior owned diagnostic did not finish'
  time.sleep(.2)
if (out/'baseline-session.log').exists():(out/'baseline-session.log').rename(out/'misdirected-comparison.log')
env=dict(os.environ,CARGO_TARGET_DIR=str(root/'.local/p14-target'),CARGO_INCREMENTAL='0')
commands={}
def run(name,args,cwd=root,allowed=(0,)):
 commands[name]={'argv':args,'cwd':str(cwd)}
 with (out/(name+'.log')).open('w') as log:
  result=subprocess.run(args,cwd=cwd,env=env,stdout=log,stderr=subprocess.STDOUT)
 commands[name]['exit_code']=result.returncode
 (out/'final-commands.json').write_text(json.dumps(commands,indent=2)+'\n')
 assert result.returncode in allowed,(name,result.returncode)
def cargo(profile,action):return ['cargo',action,'--offline','--locked','--manifest-path','product/Cargo.toml','--config',f'profile.{profile}.inherits="dev"','--config',f'profile.{profile}.debug=0','--profile',profile]
files=['product/crates/ag1-platform/'+p for p in ['src/mcp/mod.rs','src/mcp/manager.rs','src/mcp/manager/execution.rs','src/mcp/manager/prepared.rs','src/mcp/manager/selection.rs','src/session/mcp.rs','tests/mcp_execution_contract.rs','tests/mcp_content/mod.rs','tests/mcp_runtime/mod.rs']]
run('final-format',['rustfmt','--check','--edition','2024','--config','skip_children=true',*files])
run('final-contracts',cargo('mcp-b4-final','test')+['-p','ag1-core','--test','mcp_registry_contract','--test','mcp_capability_contract','-p','ag1-platform','--test','mcp_execution_contract','--test','mcp_manager_contract','--test','mcp_http_contract','--test','mcp_stdio_contract','--test','mcp_foundation_contract','--test','platform_contract','--test','github_local_contract','-p','ag1-storage','--test','mcp_storage_contract','--test','storage_contract','--test','tool_storage_contract','--','--test-threads=1'])
run('final-units',cargo('mcp-b4-final','test')+['-p','ag1-platform','--lib','mcp::','--','--test-threads=1'])
run('final-clippy',cargo('mcp-b4-final','clippy')+['-p','ag1-platform','--lib'])
run('final-session',cargo('mcp-b4-final','test')+['-p','ag1-platform','--test','session_contract','--','--test-threads=1'],allowed=(0,101))
run('baseline-session',cargo('mcp-b4-before','test')+['-p','ag1-platform','--test','session_contract','--','--test-threads=1'],cwd=out/'baseline',allowed=(0,101))
print(json.dumps(commands,indent=2))
