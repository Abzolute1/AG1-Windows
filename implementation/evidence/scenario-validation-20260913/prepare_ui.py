import os,json,subprocess,shutil,hashlib,time
from pathlib import Path
r=Path.cwd();e=r/'implementation/evidence/scenario-validation-20260913';c=r/'.local/p14-live-fixes/native-candidate-sv13';release=Path('/opt/ag1/current').resolve()
env=dict(os.environ,CARGO_TARGET_DIR=str(r/'.local/p14-target'),CARGO_BUILD_JOBS='6',AG1_LLAMA_CPP='0');env['PATH']=str(r/'.local/toolchains/node-v24.14.0-linux-x64/bin')+':'+env['PATH']
cmd=['cargo','build','--manifest-path','product/Cargo.toml','--offline','--locked','--release','-p','ag1-desktop','--bin','ag1-desktop','--features','visual-test'];t=time.monotonic()
with (e/'visual-build.log').open('x') as f: p=subprocess.run(cmd,env=env,stdout=f,stderr=subprocess.STDOUT)
(e/'visual-build.json').write_text(json.dumps({'command':cmd,'exit_code':p.returncode,'seconds':time.monotonic()-t},indent=2));p.check_returncode()
c.mkdir(mode=0o700);shutil.copy2(r/'.local/p14-target/release/ag1-desktop',c/'ag1-desktop');shutil.copy2(release/'bin/ag1-process-host',c/'ag1-process-host');shutil.copytree(r/'product/apps/desktop/dist',c/'renderer')
with (e/'native-mcp.log').open('x') as f: p=subprocess.run([str(r/'.local/toolchains/node-v24.14.0-linux-x64/bin/node'),str(e/'native-mcp-upgrade.mjs')],stdout=f,stderr=subprocess.STDOUT,env=env,timeout=120)
(e/'native-mcp-exit.json').write_text(json.dumps({'exit_code':p.returncode,'screenshots':False,'installed_binary':False,'current_source_instrumented_helper':True},indent=2));p.check_returncode()
for p in c.glob('mcp-*/result.json'):shutil.copy2(p,e/'native-mcp-result.json');shutil.copy2(p.with_name('mcp-view.json'),e/'native-mcp-view.json')
