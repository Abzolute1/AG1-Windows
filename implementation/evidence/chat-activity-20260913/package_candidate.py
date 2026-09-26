import os,sys,json,time
from pathlib import Path
r=Path.cwd();e=r/'implementation/evidence/chat-activity-20260913';sys.path.insert(0,str(r/'tools'));os.environ.update(CARGO_BUILD_JOBS='6',CARGO_INCREMENTAL='0',AG1_LLAMA_CPP='0');os.environ['PATH']=str(r/'.local/toolchains/node-v24.14.0-linux-x64/bin')+':'+os.environ['PATH']
import release_package
from release_package import package,validate_candidate
original_environment=release_package.environment
def build_environment():
 node,env=original_environment();env.update(CARGO_BUILD_JOBS="6",CARGO_INCREMENTAL="0");return node,env
release_package.environment=build_environment
from release_gate import fingerprints
before=fingerprints();report=package();(e/'package.json').write_text(json.dumps(report,indent=2)+'\n');assert before==fingerprints()
installed=validate_candidate(report,report['input_sha256'],r,r/'.local/chat-activity-install-check-20260913')
(e/'package-validation.json').write_text(json.dumps({'status':'passed','isolated_install':str(installed),'source_unchanged_during_package':True},indent=2)+'\n');print('Packaged and isolated installer passed',flush=True)
