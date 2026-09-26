from pathlib import Path
import subprocess,shutil
ROOT=Path('/home/alex/Projects/AG1');R=ROOT/'.local/ag1-optimized-validation-20260914';W=ROOT/'.local/p14-long-horizon/ag1perf-20260914';d=W/'candidate-recovery';d.mkdir(mode=0o700);(R/'candidate-recovery').mkdir(mode=0o700);shutil.copyfile(W/'candidate/config.json',d/'config.json');(d/'config.json').chmod(0o600)
prefix=['/usr/bin/bwrap','--die-with-parent','--unshare-pid','--ro-bind','/','/','--proc','/proc','--dev-bind','/dev','/dev','--bind',str(R),str(R),'--bind',str(W),str(W),'--bind',str(R/'authority'),'/home/alex/.local/state/ag1/authority-v1','--tmpfs','/tmp','--']
for args in [['--init',str(d/'workspace')],['--workspace',str(d/'workspace'),'--state',str(d/'state'),'--config',str(d/'config.json'),'--export',str(d/'initial.html')]]:
 subprocess.run(prefix+['/opt/ag1/current/bin/ag1-cli']+args,check=True,capture_output=True,timeout=120)
