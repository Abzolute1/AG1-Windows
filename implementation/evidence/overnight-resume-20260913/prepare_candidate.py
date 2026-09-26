"""Build a unique CPU-validated follow-up candidate; preserve overnight receipts."""
import json, os, shutil, subprocess, sys, time
from pathlib import Path
ROOT=Path('/home/alex/Projects/AG1'); OUT=ROOT/'implementation/evidence/overnight-resume-20260913'
os.chdir(ROOT);sys.path.insert(0,str(ROOT/'tools'))
from release_package import package, validate_candidate
from release_gate import fingerprints, ref
before=fingerprints()
report=package()
(OUT/'package.json').write_text(json.dumps(report,indent=2)+'\n')
installed=validate_candidate(report,report['input_sha256'],ROOT,ROOT/'.local/overnight-resume-20260913-install-check')
(OUT/'package-validation.json').write_text(json.dumps({'status':'passed','scope':'Fresh isolated installer validation, all archive files and manifest checked; not systemwide activation.','package':ref(OUT/'package.json'),'manifest':ref(installed/'package-manifest.json')},indent=2)+'\n')
target=ROOT/'.local/p14-target'; destination=ROOT/'.local/overnight-resume-20260913-binaries';destination.mkdir(mode=0o700)
env=os.environ.copy();env.update(CARGO_TARGET_DIR=str(target),CARGO_BUILD_JOBS='6',AG1_LLAMA_CPP='0')
rows=[]
def build(label,args):
    command=['cargo','build','--manifest-path','product/Cargo.toml','--offline','--locked','--release']+args
    log=OUT/f'{label}-build.log';start=time.monotonic()
    with log.open('x') as stream: result=subprocess.run(command,env=env,stdout=stream,stderr=subprocess.STDOUT)
    row=dict(command=command,exit_code=result.returncode,seconds=time.monotonic()-start,log=str(log));rows.append(row);print(json.dumps(row),flush=True)
    if result.returncode:raise RuntimeError('Helper build failed: '+label)
build('native-helpers',['-p','ag1-platform','--example','release-long-horizon-native','--example','release-long-horizon-owner','--example','release-recovery-checkpoint'])
for name in ['release-long-horizon-native','release-long-horizon-owner','release-recovery-checkpoint']:shutil.copy2(target/'release/examples'/name,destination/name)
build('visual-helper',['-p','ag1-desktop','--bin','ag1-desktop','--features','visual-test'])
shutil.copy2(target/'release/ag1-desktop',destination/'ag1-desktop-visual')
shutil.copy2(ROOT/'.local/overnight-20260913-run8-binaries/heap-observer.so',destination/'heap-observer.so')
if fingerprints()!=before:raise RuntimeError('Packaging/helper builds changed source inputs')
(OUT/'candidate-preparation.json').write_text(json.dumps({'status':'packaged-and-helpers-retained','package':ref(OUT/'package.json'),'binaries':[ref(p) for p in sorted(destination.iterdir())],'builds':rows,'source_fingerprints':before,'systemwide_installation':'pending','eight_hour_run':'not started'},indent=2)+'\n')
print('Candidate and helpers prepared.',flush=True)
