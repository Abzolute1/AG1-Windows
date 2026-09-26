from pathlib import Path
import json,hashlib,shutil,re
root=Path('/home/alex/Projects/AG1');base=root/'.local/mcp-c2';out=root/'implementation/evidence/MCP/authentication/oauth-native-20260912'
out.mkdir(exist_ok=False)
def sha(p):
 with p.open('rb') as f:return hashlib.file_digest(f,'sha256').hexdigest()
def save(p,v):p.write_text(json.dumps(v,indent=2)+'\n')
source=json.loads((base/'ui3-source.json').read_text())
for n,h in source.items():
 assert sha(root/n)==h,n
 dest=out/'source'/n;dest.parent.mkdir(parents=True,exist_ok=True);shutil.copy2(root/n,dest)
save(out/'source-manifest.json',source)
for p in base.iterdir():
 if p.is_file() and (p.suffix=='.log' or p.name in ['acceptance-commands.json','session-comparison.json','core-comparison.json','ui-comparison.json','baseline-sdk-inputs.json','manual-regression-candidate.json','manual-regression-candidate-2.json','preservation-native-ui1-20260912.json'] or p.name.startswith(('ui1-','ui2-','ui3-')) and p.suffix=='.json'):
  dest=out/'records'/p.name;dest.parent.mkdir(exist_ok=True);shutil.copy2(p,dest)
for p in base.glob('*.py'):
 dest=out/'drivers'/p.name;dest.parent.mkdir(exist_ok=True);shutil.copy2(p,dest)
tests=set()
for name in ['acceptance-contracts','acceptance-units','acceptance-session','acceptance-core','fixture-oauth-regression-1']:
 for n in re.findall(r'Running .*?\(([^\n)]+)\)',(base/(name+'.log')).read_text()):tests.add(n)
tests.add('.local/p14-target/mcp-c2/ag1-process-host')
for n in sorted(tests):
 p=root/n;assert p.is_file(),n;dest=out/'executables'/p.name;dest.parent.mkdir(exist_ok=True);shutil.copy2(p,dest)
native={
 'oauth':'.local/p14-live-fixes/native-candidate-mcp-c2-ui3/oa-3HGH7M',
 'manager':'.local/p14-live-fixes/native-candidate-mcp-c2-ui3/mcp-72Sw3j',
 'manual':'.local/p14-live-fixes/native-candidate-mcp-c1-c2/cred-AFzYVn',
}
for label,name in native.items():
 work=root/name;assert json.loads((work/'result.json').read_text())['status']=='passed'
 dest=out/'native'/label;dest.mkdir(parents=True)
 for p in work.iterdir():
  if p.is_file() and (p.suffix=='.png' or p.name in ['result.json','oauth-secret-scan.json','secret-scan.json','oauth-events.json']):shutil.copy2(p,dest/p.name)
protected=json.loads((base/'preservation-through-manager.json').read_text())
for n,h in protected.items():assert sha(root/n)==h,n
old_count=len(protected)
for directory in ['.local/mcp-c2/ui1-source','.local/mcp-c2/ui2-source','.local/mcp-c2/ui3-source','.local/p14-live-fixes/native-candidate-mcp-c2-ui1','.local/p14-live-fixes/native-candidate-mcp-c2-ui2','.local/p14-live-fixes/native-candidate-mcp-c2-ui3','.local/p14-live-fixes/native-candidate-mcp-c1-c2-auth-regression','.local/p14-live-fixes/native-candidate-mcp-c1-c2']:
 for p in (root/directory).rglob('*'):
  if p.is_file() and not p.is_symlink():protected[str(p.relative_to(root))]=sha(p)
files={str(p.relative_to(out)):sha(p) for p in out.rglob('*') if p.is_file()}
record={'status':'accepted_source_checkpoint','checkpoint':'C2 native OAuth with public pre-registration','full_mcp_complete':False,'production_installed':False,'source_files':len(source),'test_executables':len(tests),'selected_rust_passes':275,'fixture_feature_oauth_passes':21,'mcp_frontend_passes':10,'frontend_results':{'passed':237,'baseline_failures':2},'session_results':{'passed':243,'baseline_failures':16},'core_results':{'passed':22,'baseline_failures':6},'baseline_comparison':'same failure assertion bodies for Rust; same names and primary reasons for frontend','native':native,'native_secret_scan':{'oauth_files':47,'oauth_hits':0,'limitation':'literal synthetic access/refresh tokens; owned peer files and dedicated vault excluded'},'clippy':{'platform_warnings':64,'existing':63,'added':[{'file':'mcp/oauth/network.rs','warning':'request_checked has 8 arguments; clippy too_many_arguments','disposition':'Recorded API-shape lint; no production checks, assertions or compiler warnings suppressed.'}]},'prior_protected_verified':old_count,'limitations':['Public pre-registration only; see MCP-OAUTH-REGISTRATION.md','Diagnostic single-origin CA policy absent from normal production feature tree','No live account, inference or native capability invocation; shared execution separately covered by Rust contracts','Production package, startup and installation still required'],'failed_runs_retained':['UI2 rejected-refresh failure exposed stale renderer state; driver also incorrectly attempted refresh after the native reservation was revoked','First typecheck used wrong working directory','First manual regression candidate exceeded Unix socket path limit before launch','First reconstructed baseline UI omitted SDK files; restored 13 files verified unchanged against retained baseline'],'files':files}
save(out/'manifest.json',record)
for p in out.rglob('*'):
 if p.is_file():protected[str(p.relative_to(root))]=sha(p)
save(base/'preservation-through-native.json',protected)
save(base/'native-preservation-final.json',{'verified_previous':old_count,'protected_now':len(protected),'mismatches':[]})
save(base/'acceptance-ready.json',{'status':'source_accepted_pending_production','manifest':str(out/'manifest.json'),'sha256':sha(out/'manifest.json'),'protected_manifest':str(base/'preservation-through-native.json'),'source_manifest':str(out/'source-manifest.json')})
print(json.dumps({'source_files':len(source),'executables':len(tests),'protected':len(protected),'out':str(out)}),flush=True)
