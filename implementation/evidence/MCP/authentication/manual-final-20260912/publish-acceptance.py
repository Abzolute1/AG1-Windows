from pathlib import Path
import json,hashlib,re,shutil,subprocess
root=Path('/home/alex/Projects/AG1');scratch=root/'.local/mcp-c1';out=root/'implementation/evidence/MCP/authentication/manual-final-20260912'
def sha(p):
 p=Path(p);p=p if p.is_absolute() else root/p
 return hashlib.file_digest(p.open('rb'),'sha256').hexdigest()
def write(p,v):p.write_text(json.dumps(v,indent=2)+'\n')
def relative(p):return str(p.relative_to(root))
commands=json.loads((scratch/'final-commands.json').read_text());verified=json.loads((scratch/'final-source-verification.json').read_text());assert verified['unchanged']
for name in ['final-contracts','final-units','final-clippy','final-typecheck','final-renderer','final-desktop','final-native-ui','final-private-native-ui']:assert commands[name]['exit_code']==0,name
comparisons={}
for suite in ['session','core']:
 v=json.loads((scratch/f'{suite}-comparison.json').read_text());assert v['identical_assertion_bodies'] and v['baseline_results']==v['candidate_results'];comparisons[suite]=v
comparisons['ui']=json.loads((scratch/'ui-comparison.json').read_text());assert comparisons['ui']['same_failure_names_and_reasons']
frozen=json.loads((scratch/'frozen-source.json').read_text());assert all(sha(p)==h for p,h in frozen.items())
protected=json.loads((scratch/'preservation-start.json').read_text());assert all(sha(p)==h for p,h in protected.items())
before=json.loads((scratch/'before-sha256.json').read_text())
registration={
 'product/crates/ag1-platform/tests/mcp_execution_contract.rs':'#[path = "mcp_private_execution/mod.rs"]\nmod private_execution;\n',
 'product/crates/ag1-platform/tests/mcp_http_contract.rs':'#[path = "mcp_http/authenticated.rs"]\nmod authenticated;\n',
 'product/crates/ag1-platform/tests/mcp_manager_contract.rs':'#[path = "mcp_manager/private_credentials.rs"]\nmod private_credentials;\n'}
for name in before:
 if '/tests/' not in name and '.test.' not in name:continue
 a=(scratch/'before'/name).read_bytes();b=(root/name).read_bytes()
 if name.endswith('/MCPManager.test.tsx'):assert b.startswith(a),name
 elif name in registration:assert b.replace(registration[name].encode(),b'',1)==a,name
 else:assert a==b,name
results={};binaries={}
for logfile in ['final-contracts.log','final-units.log','final-session.log','baseline-session.log','final-core.log','baseline-core.log']:
 s=(scratch/logfile).read_text()
 for p in re.findall(r'Running [^\n]*\(([^\n)]+)\)',s):binaries[p]=sha(p)
 for name,passed,failed in re.findall(r'Running (?:unittests |tests/)?(\S+?)(?:\.rs)? \([^\n]+\).*?test result: (?:ok|FAILED)\. (\d+) passed; (\d+) failed',s,re.S):results[logfile+':'+name]={'passed':int(passed),'failed':int(failed)}
selected=sum(v['passed'] for k,v in results.items() if k.startswith(('final-contracts.log:','final-units.log:')));assert selected==253,selected
inspection=json.loads((scratch/'native-inspection.json').read_text());assert inspection['status']=='inspected'
assert not out.exists();out.mkdir(parents=True)
visuals={}
for label,candidate,prefix in [('manager',root/'.local/p14-live-fixes/native-candidate-mcp-c1-final','mcp-'),('credentials',root/'.local/p14-live-fixes/native-candidate-mcp-c1-final','cred-'),('bound-helper',root/'.local/p14-live-fixes/native-candidate-mcp-c1-boundhelper','cred-')]:
 found=list(candidate.glob(prefix+'*/result.json'));assert len(found)==1,(label,found)
 result=json.loads(found[0].read_text());assert result['status']=='passed' and result['providerRequests']==[] and result['sha256']==sha(candidate/'ag1-desktop')
 for name in ['ag1-desktop','ag1-process-host']:binaries[relative(candidate/name)]=sha(candidate/name)
 dest=out/label;dest.mkdir()
 for p in found[0].parent.iterdir():
  if p.is_file() and (p.suffix=='.png' or p.name in ['result.json','desktop.log','private-peer-events.json']):shutil.copy2(p,dest/p.name)
 visuals[label]={'result':relative(dest/'result.json'),'desktop_sha256':sha(candidate/'ag1-desktop'),'helper_sha256':sha(candidate/'ag1-process-host')}
 assert label in inspection['workflows']
for name in ['final-commands.json','final-checks.py','compare.py','session-comparison.json','core-comparison.json','ui-comparison.json','before-sha256.json','frozen-source.json','source-delta.json','source-delta.patch','final-source-verification.json','native-inspection.json','bound-helper-input.json','publish-acceptance.py']:
 shutil.copy2(scratch/name,out/name)
for p in scratch.glob('final-*.log'):shutil.copy2(p,out/p.name)
for p in scratch.glob('baseline-*.log'):shutil.copy2(p,out/p.name)
shutil.copy2(scratch/'baseline/baseline.json',out/'baseline.json')
write(out/'preservation.json',{'files':protected,'unchanged':True})
report={'date':'2026-09-12','checkpoint':'C1 manual private credentials; phase C partial','status':'accepted_source_checkpoint','full_mcp_complete':False,'production_installed':False,'source_manifest':{'path':relative(out/'frozen-source.json'),'sha256':sha(out/'frozen-source.json'),'count':len(frozen)},'commands':relative(out/'final-commands.json'),'results':results,'selected_total_passed':selected,'new_rust_checks':25,'new_ui_checks':2,'baseline_comparisons':comparisons,'binary_sha256':binaries,'native_workflows':visuals,'preservation':{'path':relative(out/'preservation.json'),'files':len(protected),'unchanged':True},'frozen_test_assertions_preserved':True,'compiler':subprocess.check_output(['rustc','--version'],text=True).strip(),'Cargo_lock_sha256':sha('product/Cargo.lock'),'clippy':'Exit 0, same 63 existing platform warnings','scope':'Dedicated private store and vault, manual HTTP/local private entry, manager/session/configuration/generation binding, logout, bounded injection and literal secret-echo withholding','limits':['Manual credentials are locally private; no OS-keychain encryption, locked input buffers or forensic SQLite/WAL erasure claim.','Literal supplied-secret echo checks do not detect arbitrary transformed exfiltration.','OAuth discovery/browser/refresh and reviewed scoped trust controls are unfinished.','Public registry discovery/guided setup, supported input continuation, media, delegation and full A-F acceptance remain.','No live account, model inference or paid effect was used.','Normal production packaging/install follows this source acceptance.']}
report['artifacts']={relative(p):sha(p) for p in out.rglob('*') if p.is_file()}
write(root/'implementation/evidence/MCP/authentication/manual-acceptance.json',report)
write(scratch/'acceptance-ready.json',{'checkpoint':'C1 manual credentials','report':'implementation/evidence/MCP/authentication/manual-acceptance.json','selected_passes':selected,'full_mcp_complete':False,'production_installation_pending':True})
print(json.dumps({'accepted':'C1 source checkpoint','selected_passes':selected,'source_files':len(frozen),'preserved_files':len(protected)}))
