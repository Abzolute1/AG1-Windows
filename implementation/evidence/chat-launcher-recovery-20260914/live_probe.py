from pathlib import Path
import subprocess,os,signal,time,json,hashlib
root=Path('/home/alex/Projects/AG1');out=root/'implementation/evidence/chat-launcher-recovery-20260914';scratch=root/'.local/chat-launcher-recovery-20260914'
command=['/opt/ag1/current/bin/ag1-cli','--workspace',str(scratch/'workspace'),'--state',str(scratch/'state'),'--config',str(scratch/'probe-config.json'),'--print','--trust-workspace','--no-tools','--max-turns','3','--','Reply with exactly: AG1_SEND_RECOVERY_OK']
start=time.monotonic();timeout=False
with (out/'live-send.stdout').open('w') as stdout,(out/'live-send.stderr').open('w') as stderr:
 p=subprocess.Popen(command,cwd=root,stdout=stdout,stderr=stderr,stdin=subprocess.DEVNULL,start_new_session=True)
 try:p.wait(timeout=120)
 except subprocess.TimeoutExpired:
  timeout=True;os.killpg(p.pid,signal.SIGTERM)
  try:p.wait(timeout=10)
  except subprocess.TimeoutExpired:os.killpg(p.pid,signal.SIGKILL);p.wait()
answer=(out/'live-send.stdout').read_text().strip();result={'exitCode':p.returncode,'timedOut':timeout,'elapsedSeconds':time.monotonic()-start,'answer':answer,'passed':p.returncode==0 and answer=='AG1_SEND_RECOVERY_OK','scope':'Installed native CLI, real 35B, copied desktop configuration and isolated conversation storage; no screen capture'}
(out/'live-send.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result));assert result['passed']
