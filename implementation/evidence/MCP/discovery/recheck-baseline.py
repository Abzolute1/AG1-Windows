from pathlib import Path
import json,subprocess,os
root=Path('/home/alex/Projects/AG1');out=root/'.local/mcp-b5';commands=json.loads((out/'final-commands.json').read_text())
commands['baseline-session-transient']=commands.pop('baseline-session')
(out/'baseline-session.log').rename(out/'baseline-session-transient.log')
original=commands['baseline-session-transient']
argv=original['argv']
env=dict(os.environ,CARGO_TARGET_DIR=str(root/'.local/p14-target'),CARGO_INCREMENTAL='0')
for name,args in [('baseline-extension-recheck',argv[:-2]+['extensions::tools::recovery_in_extension_runs_uses_native_results_not_display_claims','--','--exact','--test-threads=1']),('baseline-session',argv)]:
 with (out/(name+'.log')).open('w') as log:r=subprocess.run(args,cwd=original['cwd'],env=env,stdout=log,stderr=subprocess.STDOUT)
 commands[name]={'argv':args,'cwd':original['cwd'],'exit_code':r.returncode}
 (out/'final-commands.json').write_text(json.dumps(commands,indent=2)+'\n')
 print(name,r.returncode,flush=True)
 assert r.returncode in ([0] if name.endswith('recheck') else [0,101])
