import sys,shutil,json
from pathlib import Path
sys.path.insert(0,str(Path('tools').resolve()))
import release_package,release_gate
from common import ROOT
output=ROOT/'implementation/evidence/free-search-recovery-20260914'
shutil.copyfile(release_gate.EVIDENCE/'accepted-entry.json',output/'accepted-entry.json')
release_gate.EVIDENCE=output
original=release_package.environment
def environment():
 node,env=original();env.update(CARGO_INCREMENTAL='0',CARGO_BUILD_JOBS='6');return node,env
release_package.environment=environment
report=release_package.package()
installed=release_package.validate_candidate(report,report['input_sha256'],ROOT,ROOT/'.local/free-search-recovery-20260914/install-check')
(output/'package-validation.json').write_text(json.dumps({'status':'passed','installed':str(installed),'archive':report['archive'],'manifestFiles':len(json.loads((installed/'package-manifest.json').read_text())['files'])},indent=2)+'\n')
print(json.dumps({'archive':report['archive'],'installed':str(installed)},indent=2))
