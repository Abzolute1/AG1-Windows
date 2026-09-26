import json,os,subprocess,sys,time
from pathlib import Path
ROOT=Path('/home/alex/Projects/AG1');OUT=ROOT/'implementation/evidence/overnight-final-followup-20260913'
os.chdir(ROOT);sys.path.insert(0,str(ROOT/'tools'))
from release_package import package,validate_candidate
from release_gate import fingerprints,ref
before=fingerprints();env=os.environ.copy();env.update(CARGO_TARGET_DIR=str(ROOT/'.local/p14-target'),CARGO_BUILD_JOBS='6',AG1_LLAMA_CPP='0')
# Native session pass is retained before the final facade-only change; the
# complete native_surface_contract target passed after that change.
report=package();(OUT/'package.json').write_text(json.dumps(report,indent=2)+'\n')
installed=validate_candidate(report,report['input_sha256'],ROOT,ROOT/'.local/overnight-final-followup-install-check-v4')
(OUT/'package-validation.json').write_text(json.dumps({'status':'passed','scope':'Isolated checksum-verifying installer','manifest':ref(installed/'package-manifest.json')},indent=2)+'\n')
if fingerprints()!=before:raise RuntimeError('Source inputs changed during final build')
print('Final candidate packaged and validated.',flush=True)
