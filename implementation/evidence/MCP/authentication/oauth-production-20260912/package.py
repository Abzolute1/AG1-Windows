import sys,json,shutil
from pathlib import Path
sys.path.insert(0,'tools')
import release_package,release_gate
base=Path('.local/mcp-c2')
profile='mcp-c2-production-20260912'
assert not Path('.local/p14-target').joinpath(profile).exists(), 'Use a fresh production profile'
assert Path('implementation/evidence/MCP/authentication/oauth-native-20260912/manifest.json').exists(), 'Frozen source acceptance required'
assert Path('.local/mcp-c2/acceptance-ready.json').exists(), 'Review final evidence before packaging'
original_environment=release_package.environment
original_run=release_package.run
original_copy=shutil.copyfile
old_evidence=release_gate.EVIDENCE
release_gate.EVIDENCE=(base/'package-evidence').resolve()
release_gate.EVIDENCE.mkdir(exist_ok=True)
original_copy(old_evidence/'accepted-entry.json',release_gate.EVIDENCE/'accepted-entry.json')
def environment():
 node,env=original_environment();env['CARGO_INCREMENTAL']='0';return node,env
def run(command,**kwargs):
 if command[:2]==['cargo','build']:
  command=list(command);i=command.index('--release');command[i:i+1]=['--config',f'profile.{profile}.inherits="release"','--profile',profile]
 return original_run(command,**kwargs)
def copy(source,destination,*args,**kwargs):
 path=Path(source)
 if path.parent==release_package.ROOT/'.local/p14-target/release' and path.name in release_package.BINARIES:
  source=path.parent.parent/profile/path.name
 return original_copy(source,destination,*args,**kwargs)
release_package.environment=environment
release_package.run=run
shutil.copyfile=copy
report=release_package.package()
report['production_profile']=profile
(base/'package.json').write_text(json.dumps(report,indent=2))
print(json.dumps({'status':report['status'],'archive':report['archive'],'input_sha256':report['input_sha256']}),flush=True)
