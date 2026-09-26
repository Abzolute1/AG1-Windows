from pathlib import Path
import os,subprocess,json,hashlib,shutil
root=Path('/home/alex/Projects/AG1');out=root/'.local/mcp-c2';candidate=root/'.local/p14-live-fixes/native-candidate-mcp-c2-ui2'
candidate.mkdir(exist_ok=False)
def sha(p):
 with p.open('rb') as f:return hashlib.file_digest(f,'sha256').hexdigest()
paths=set(json.loads((out/'before-sha256.json').read_text()))
for directory in ['product/crates/ag1-platform','product/apps/desktop/src','product/apps/desktop/src-tauri/src']:
 paths.update(str(p.relative_to(root)) for p in (root/directory).rglob('*') if p.is_file() and p.suffix in ['.rs','.py','.json','.ts','.tsx','.css','.toml'])
paths.update(['product/apps/desktop/src-tauri/Cargo.toml','product/Cargo.lock','tools/probes/mcp_oauth_peer.py','tools/probes/mcp_oauth_browser.py','tools/probes/mcp_oauth_input.py','tools/probes/p14_native_startup_smoke.mjs','tools/probes/mcp_native_oauth_audit.mjs'])
frozen={p:sha(root/p) for p in sorted(paths)}
(out/'ui2-source.json').write_text(json.dumps(frozen,indent=2)+'\n')
for name in frozen:
 dest=out/'ui2-source'/name;dest.parent.mkdir(parents=True,exist_ok=True);shutil.copy2(root/name,dest)
env=dict(os.environ,CARGO_TARGET_DIR=str(root/'.local/p14-target'),CARGO_INCREMENTAL='0')
node=str(root/'.local/toolchains/node-v24.14.0-linux-x64/bin/node')
with (out/'ui2-renderer-build.log').open('w') as log:
 subprocess.run([node,'node_modules/vite/bin/vite.js','build','--outDir',str(candidate/'renderer')],cwd=root/'product/apps/desktop',env=env,stdout=log,stderr=subprocess.STDOUT,check=True)
env['TAURI_CONFIG']=json.dumps({'build':{'frontendDist':str(candidate/'renderer')}})
with (out/'ui2-desktop-build.log').open('w') as log:
 subprocess.run(['cargo','build','--offline','--locked','--manifest-path','product/Cargo.toml','--config','profile.mcp-c2.inherits="dev"','--config','profile.mcp-c2.debug=0','--profile','mcp-c2','-p','ag1-desktop','-p','ag1-platform','--bin','ag1-desktop','--bin','ag1-process-host','--features','ag1-desktop/visual-test'],cwd=root,env=env,stdout=log,stderr=subprocess.STDOUT,check=True)
for name,h in frozen.items():assert sha(root/name)==h,name
for name in ['ag1-desktop','ag1-process-host']:shutil.copy2(root/'.local/p14-target/mcp-c2'/name,candidate/name)
report={'status':'built_pending_native_audit','candidate':str(candidate),'source_manifest':str(out/'ui2-source.json'),'binaries':{name:sha(candidate/name) for name in ['ag1-desktop','ag1-process-host']}}
(out/'ui2-candidate.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report),flush=True)
