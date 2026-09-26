import os,subprocess,shutil,json,time
from pathlib import Path
r=Path.cwd();e=r/'implementation/evidence/chat-activity-20260913';env=dict(os.environ,CARGO_TARGET_DIR=str(r/'.local/p14-target'),CARGO_BUILD_JOBS='6',CARGO_INCREMENTAL='0',AG1_LLAMA_CPP='0');env['PATH']=str(r/'.local/toolchains/node-v24.14.0-linux-x64/bin')+':'+env['PATH']
# Wait only for the owned package receipt; no independent release or user process is touched.
deadline=time.monotonic()+600
while not (e/'package-validation.json').exists():
 if time.monotonic()>deadline:raise TimeoutError('Owned production package did not finish')
 time.sleep(1)
with (e/'visual-final-build.log').open('x') as log:subprocess.run(['cargo','build','--manifest-path','product/Cargo.toml','--release','--offline','--locked','-p','ag1-desktop','--bin','ag1-desktop','--features','visual-test'],env=env,stdout=log,stderr=subprocess.STDOUT,check=True)
c=r/'.local/p14-live-fixes/native-candidate-chat13-final';c.mkdir(mode=0o700);shutil.copy2(r/'.local/p14-target/release/ag1-desktop',c/'ag1-desktop');shutil.copy2(r/'.local/p14-target/release/ag1-process-host',c/'ag1-process-host');shutil.copytree(r/'product/apps/desktop/dist',c/'renderer')
print('Native visual helper ready',flush=True)
