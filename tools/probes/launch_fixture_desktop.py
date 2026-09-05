"""Restart only the owned native fixture. Notify the user before invoking this probe."""
import hashlib, json, os, subprocess, time
from pathlib import Path
from desktop_visual import js
ROOT=Path(__file__).resolve().parents[2]
def main():
 base=ROOT/'.local/p04-visual';base.mkdir(parents=True,exist_ok=True)
 record_file=base/'process.json'
 if record_file.exists():
  record=json.loads(record_file.read_text());proc=Path(f'/proc/{record["pid"]}/stat')
  if proc.exists():
   assert proc.read_text().rsplit(') ',1)[1].split()[19]==record['start'],'PID reused; refusing to stop'
   try:js('window.__TAURI_INTERNALS__.invoke("quit");true')
   except (ConnectionError,RuntimeError,OSError):pass
   for _ in range(200):
    if not proc.exists():break
    time.sleep(.05)
   assert not proc.exists(),'Prior owned desktop has not exited'
 (base/'control.sock').unlink(missing_ok=True)
 env=os.environ.copy();env.update(HOME=str(base/'home'),GDK_BACKEND='x11',WEBKIT_DISABLE_DMABUF_RENDERER='1',AG1_INSTALL_ROOT=str(ROOT),AG1_DESKTOP_STATE=str(base/'state'),AG1_FIXTURE_PROVIDER='1',AG1_VISUAL_SOCKET=str(base/'control.sock'))
 with (ROOT/'implementation/evidence/P04/ag1-visual-working.log').open('a') as log:
  start=time.monotonic();child=subprocess.Popen([str(ROOT/'product/target/release/ag1-desktop')],env=env,stdout=log,stderr=log,start_new_session=True)
 record={'pid':child.pid,'start':Path(f'/proc/{child.pid}/stat').read_text().rsplit(') ',1)[1].split()[19],'launch_monotonic':start};record_file.write_text(json.dumps(record))
 deadline=time.monotonic()+20
 while time.monotonic()<deadline:
  try:
   if js('document.body.innerText.includes("What will you work on?")'):break
  except OSError:pass
  time.sleep(.02)
 else:raise RuntimeError('Packaged UI startup timeout')
 record['startup_ms']=(time.monotonic()-start)*1000
 record['binary_sha256']=hashlib.sha256((ROOT/'product/target/release/ag1-desktop').read_bytes()).hexdigest()
 record['method']='Packaged native launch to first home heading via private test broker; warm OS cache, 20 ms polling resolution.'
 record_file.write_text(json.dumps(record));(ROOT/'implementation/evidence/P04/ag1-startup.json').write_text(json.dumps(record,indent=2));print(json.dumps(record))
if __name__=='__main__':main()
