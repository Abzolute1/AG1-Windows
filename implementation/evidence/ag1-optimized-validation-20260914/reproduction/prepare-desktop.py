import json,os,shutil,subprocess
from pathlib import Path
ROOT=Path('/home/alex/Projects/AG1');R=ROOT/'.local/ag1-optimized-validation-20260914';W=ROOT/'.local/p14-long-horizon/ag1perf-20260914';W.mkdir(mode=0o700)
(R/'authority').mkdir(mode=0o700);(R/'app').mkdir();shutil.copy2(ROOT/'.local/p14-target/release/ag1-desktop',R/'app/ag1-desktop');shutil.copy2('/opt/ag1/current/bin/ag1-process-host',R/'app/ag1-process-host')
# The custom-protocol build embeds the current production renderer; preserve its exact input files too.
shutil.copytree(ROOT/'product/apps/desktop/dist',R/'app/renderer')
prefix=['/usr/bin/bwrap','--die-with-parent','--unshare-pid','--ro-bind','/','/','--proc','/proc','--dev-bind','/dev','/dev','--bind',str(R),str(R),'--bind',str(W),str(W),'--bind',str(R/'authority'),'/home/alex/.local/state/ag1/authority-v1','--tmpfs','/tmp','--']
for arm in ['original','candidate']:
 d=W/arm;d.mkdir(mode=0o700);shutil.copyfile(R/arm/'config.json',d/'config.json');(d/'config.json').chmod(0o600)
 for name,args in [('init',['--init',str(d/'workspace')]),('admit',['--workspace',str(d/'workspace'),'--state',str(d/'state'),'--config',str(d/'config.json'),'--export',str(d/'initial.html')])]:
  p=subprocess.run(prefix+['/opt/ag1/current/bin/ag1-cli']+args,capture_output=True,timeout=240)
  (R/arm/(name+'.stdout')).write_bytes(p.stdout);(R/arm/(name+'.stderr')).write_bytes(p.stderr)
  assert p.returncode==0,(arm,name,p.stderr.decode()[:300])
 print('Desktop workspace admitted',arm,flush=True)
