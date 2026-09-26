"""Grouped follow-up for the twelve targets from the retained broad failure."""
import hashlib, json, os, shutil, subprocess, time
from pathlib import Path
ROOT = Path('/home/alex/Projects/AG1')
OUT = ROOT / 'implementation/evidence/overnight-20260913'
target = ROOT / '.local/p14-target'
helper = target / 'release/ag1-process-host'
staged = target / 'release/deps/ag1-process-host'
shutil.copy2(helper, staged)
(OUT / 'test-helper-layout.json').write_text(json.dumps({
    'source': str(helper), 'test_sibling': str(staged),
    'sha256': hashlib.sha256(helper.read_bytes()).hexdigest(),
    'reason': 'NativeClient uses the process-host sibling; Cargo integration tests execute in deps.'
}, indent=2)+'\n')
env = os.environ.copy()
env.update(CARGO_TARGET_DIR=str(target), CARGO_BUILD_JOBS='6')
env['PATH'] = str(ROOT / '.local/toolchains/node-v24.14.0-linux-x64/bin')+':'+env['PATH']
cases = [
 ('core-guidance', ['-p','ag1-core','--test','task_guidance_contract']),
 ('desktop-contracts', ['-p','ag1-desktop','--test','bridge_contract','--test','cutover_contract']),
 ('platform-lib', ['-p','ag1-platform','--lib']),
 ('platform-contracts', ['-p','ag1-platform'] + sum((['--test', n] for n in [
  'client_task_guidance_contract','mcp_execution_contract','mcp_http_contract',
  'mcp_manager_contract','project_runtime_contract','session_contract',
  'tool_contract','web_http_contract']), [])),
]
rows = []
for name, args in cases:
    command = ['cargo','test','--manifest-path','product/Cargo.toml','--offline',
               '--locked','--release','--no-fail-fast']+args+['--','--test-threads=1']
    log = OUT / ('serial-failed-targets-'+name+'.log')
    started = time.monotonic()
    with log.open('x') as stream:
        result = subprocess.run(command, cwd=ROOT, env=env, stdout=stream, stderr=subprocess.STDOUT)
    row = dict(case=name,command=command,exit_code=result.returncode,
               seconds=time.monotonic()-started,log=str(log))
    rows.append(row)
    (OUT/'serial-failed-targets-results.json').write_text(json.dumps(rows,indent=2)+'\n')
    print(json.dumps(row),flush=True)
raise SystemExit(any(r['exit_code'] for r in rows))
