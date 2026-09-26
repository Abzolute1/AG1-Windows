"""Package the validated candidate, then retain separate duration helper binaries."""
import hashlib, json, os, re, shutil, subprocess, sys, time
from pathlib import Path
ROOT=Path('/home/alex/Projects/AG1')
OUT=ROOT/'implementation/evidence/overnight-20260913'
os.chdir(ROOT)
validation=json.loads((OUT/'serial-failed-targets-results.json').read_text())
followup=json.loads((OUT/'serial-platform-contracts-macro-fixed.json').read_text())
tool_followup=json.loads((OUT/'serial-tool-contract-error-fixture-fixed.json').read_text())
log=(ROOT/followup['log']).read_text()
passed_targets=re.findall(r'test result: ok\.',log)
failed_targets=re.findall(r'test result: FAILED\.',log)
expected_failure='error: 1 target failed:\n    `-p ag1-platform --test tool_contract`'
if (len(validation)!=4 or any(r['exit_code'] for r in validation[:3])
        or followup['exit_code']!=101 or len(passed_targets)!=7
        or len(failed_targets)!=1 or expected_failure not in log
        or tool_followup['exit_code']!=0):
    raise RuntimeError('Grouped remaining targets have not passed')
additional=json.loads((OUT/'post-run7-additional-changes.json').read_text())
for row in additional['files']:
    if hashlib.sha256((ROOT/row['path']).read_bytes()).hexdigest()!=row['sha256']:
        raise RuntimeError('Candidate declaration changed: '+row['path'])
sys.path.insert(0,str(ROOT/'tools'))
from release_package import package
from release_gate import fingerprints,ref
before=fingerprints()
package()
shutil.copy2(ROOT/'implementation/evidence/P14/package.json',OUT/'package-run8.json')
target=ROOT/'.local/p14-target'
destination=ROOT/'.local/overnight-20260913-run8-binaries'
destination.mkdir(mode=0o700)
env=os.environ.copy()
env.update(CARGO_TARGET_DIR=str(target),CARGO_BUILD_JOBS='6')
rows=[]
def build(label,args):
    command=['cargo','build','--manifest-path','product/Cargo.toml','--offline','--locked','--release']+args
    log=OUT/f'run8-{label}-build.log';start=time.monotonic()
    with log.open('x') as stream:
        result=subprocess.run(command,env=env,stdout=stream,stderr=subprocess.STDOUT)
    row=dict(command=command,exit_code=result.returncode,seconds=time.monotonic()-start,log=str(log))
    rows.append(row);print(json.dumps(row),flush=True)
    if result.returncode:raise RuntimeError('Helper build failed: '+label)
build('native-helpers',['-p','ag1-platform','--example','release-long-horizon-native',
      '--example','release-long-horizon-owner','--example','release-recovery-checkpoint'])
for name in ['release-long-horizon-native','release-long-horizon-owner','release-recovery-checkpoint']:
    shutil.copy2(target/'release/examples'/name,destination/name)
build('visual-helper',['-p','ag1-desktop','--bin','ag1-desktop','--features','visual-test'])
shutil.copy2(target/'release/ag1-desktop',destination/'ag1-desktop-visual')
shutil.copy2(ROOT/'.local/overnight-20260913-binaries/heap-observer.so',destination/'heap-observer.so')
if fingerprints()!=before:raise RuntimeError('Packaging/helper builds changed product inputs')
(OUT/'run8-candidate-preparation.json').write_text(json.dumps({
    'status':'packaged-and-helpers-retained','package':ref(OUT/'package-run8.json'),
    'binaries':[ref(p) for p in sorted(destination.iterdir())],
    'builds':rows,'source_fingerprints':before,
    'systemwide_installation':'pending','eight_hour_run':'not started'
},indent=2)+'\n')
print('Candidate and separate helpers prepared; systemwide installation is next.',flush=True)
