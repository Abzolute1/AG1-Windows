"""Final fix-run package, isolated checksum validation, and native UI helper."""
import json, os, shutil, subprocess, sys
from pathlib import Path
ROOT=Path('/home/alex/Projects/AG1'); OUT=ROOT/'implementation/evidence/overnight-fixrun-20260913'
os.chdir(ROOT);sys.path.insert(0,str(ROOT/'tools'))
from release_package import package, validate_candidate
from release_gate import fingerprints, ref
guard_env=os.environ.copy();guard_env.update(CARGO_TARGET_DIR=str(ROOT/'.local/p14-target'),CARGO_BUILD_JOBS='6')
with (OUT/'final-compression-closure.log').open('w') as log:
 subprocess.run(['cargo','test','--manifest-path','product/Cargo.toml','--release','--locked','-p','ag1-storage','--lib','blob_codec::tests','--','--test-threads=1'],env=guard_env,stdout=log,stderr=subprocess.STDOUT,check=True)
before=fingerprints()
report=package()
(OUT/'package.json').write_text(json.dumps(report,indent=2)+'\n')
installed=validate_candidate(report,report['input_sha256'],ROOT,ROOT/'.local/overnight-fixrun-20260913-install-check')
(OUT/'package-validation.json').write_text(json.dumps({'status':'passed','scope':'Isolated installer validation','package':ref(OUT/'package.json'),'manifest':ref(installed/'package-manifest.json')},indent=2)+'\n')
env=os.environ.copy();env.update(CARGO_TARGET_DIR=str(ROOT/'.local/p14-target'),CARGO_BUILD_JOBS='6',AG1_LLAMA_CPP='0')
with (OUT/'visual-helper-build.log').open('w') as log:
 subprocess.run(['cargo','build','--manifest-path','product/Cargo.toml','--offline','--locked','--release','-p','ag1-desktop','--bin','ag1-desktop','--features','visual-test'],env=env,stdout=log,stderr=subprocess.STDOUT,check=True)
fixture=ROOT/'.local/p14-live-fixes/native-candidate-f9';fixture.mkdir(mode=0o700)
shutil.copy2(ROOT/'.local/p14-target/release/ag1-desktop',fixture/'ag1-desktop')
shutil.copy2(installed/'bin/ag1-process-host',fixture/'ag1-process-host')
shutil.copytree(ROOT/'product/apps/desktop/dist',fixture/'renderer')
if fingerprints()!=before:raise RuntimeError('Packaging changed source inputs')
(OUT/'candidate-preparation.json').write_text(json.dumps({'status':'packaged','package':ref(OUT/'package.json'),'fixture':str(fixture),'eight_hour_run':'not started'},indent=2)+'\n')
print('Final package and native helper prepared.',flush=True)
