from pathlib import Path
import json,hashlib,re,shutil,subprocess
root=Path('/home/alex/Projects/AG1');scratch=root/'.local/mcp-b6';out=root/'implementation/evidence/MCP/desktop';assert not out.exists();out.mkdir(parents=True)
def sha(p): return hashlib.sha256((root/p if not Path(p).is_absolute() else Path(p)).read_bytes()).hexdigest()
def write(p,v): Path(p).write_text(json.dumps(v,indent=2,sort_keys=True)+'\n')
def relative(p): return str(p.relative_to(root))
commands=json.loads((scratch/'final-commands.json').read_text());verified=json.loads((scratch/'final-source-verification.json').read_text());assert verified['unchanged']
for name in ['final-contracts','final-units','final-clippy','final-typecheck','final-renderer','final-desktop','final-native-ui']: assert commands[name]['exit_code']==0
comparisons={}
for suite in ['session','core']:
 v=json.loads((scratch/f'{suite}-comparison.json').read_text());assert v['identical_assertion_bodies'] and v['baseline_results']==v['candidate_results'];comparisons[suite]={k:v[k] for k in ['baseline_results','candidate_results','identical_assertion_bodies']}
ui=json.loads((scratch/'ui-comparison.json').read_text());assert ui['same_failure_names_and_reasons'];comparisons['ui']=ui
assert not (scratch/'final-format.log').read_text().strip()
frozen=json.loads((scratch/'frozen-source.json').read_text());assert all(sha(p)==h for p,h in frozen.items())
protected=json.loads((scratch/'preservation-start.json').read_text());assert all(sha(p)==v['expected'] for p,v in protected.items())
write(out/'preservation.json',{p:{'expected':v['expected'],'actual':sha(p),'unchanged':True} for p,v in protected.items()})
write(out/'source-sha256.json',frozen)
results={};binaries={}
for logfile in ['final-contracts.log','final-units.log','final-session.log','baseline-session.log','final-core.log','baseline-core.log']:
 s=(scratch/logfile).read_text()
 for p in re.findall(r'Running [^\n]*\(([^\n)]+)\)',s): binaries[p]=sha(p)
 for name,passed,failed in re.findall(r'Running (?:unittests |tests/)?(\S+?)(?:\.rs)? \([^\n]+\).*?test result: (?:ok|FAILED)\. (\d+) passed; (\d+) failed',s,re.S): results[logfile+':'+name]={'passed':int(passed),'failed':int(failed)}
selected=sum(v['passed'] for k,v in results.items() if k.startswith(('final-contracts.log:','final-units.log:')));assert selected==228,selected
candidate=root/'.local/p14-live-fixes/native-candidate-mcp-b6-final'
visual_results=list(candidate.glob('mcp-*/result.json'));assert len(visual_results)==1
visual=json.loads(visual_results[0].read_text());assert visual['status']=='passed' and visual['providerRequests']==[] and visual['sha256']==sha(candidate/'ag1-desktop')
for name in ['ag1-desktop','ag1-process-host']:binaries[relative(candidate/name)]=sha(candidate/name)
visual_out=out/'native';visual_out.mkdir()
visual_work=visual_results[0].parent
for p in visual_work.iterdir():
 if p.is_file() and p.suffix in ['.png','.json','.log']:shutil.copy2(p,visual_out/p.name)
write(visual_out/'inspection.json',{'status':'inspected','engine':'Linux Tauri/WebKit on authenticated private Xvfb','wide':[1440,900],'narrow':[900,600],'inspected':['mcp-local-review.png','mcp-http-tools.png','mcp-local-detail-900.png','mcp-local-900.png'],'findings':['Literal peer markup is text','Local host-account launch warning and exact command review','Generation numbers displayed as exact decimal strings','Narrow server list and detail use one column','Inspection-only execution limitation remains visible'],'limits':['DOM input, not physical-input acceptance','Not the production installation; production build follows']})
helper=json.loads((scratch/'bound-helper-input.json').read_text());assert sha(helper['helper'])==helper['sha256']
helper_result=list((root/'.local/p14-live-fixes/native-candidate-mcp-b6-boundhelper').glob('mcp-*/result.json'));assert len(helper_result)==1
helper_visual=json.loads(helper_result[0].read_text());assert helper_visual['status']=='passed' and helper_visual['providerRequests']==[]
write(out/'bound-helper-compatibility.json',{'input':helper,'result':helper_visual,'test_scope':'Owned unauthenticated local/remote manager lifecycle with the existing bound process helper; preliminary desktop binary, not final source acceptance'})
for p in scratch.glob('*.log'):shutil.copy2(p,out/p.name)
for name in ['final-commands.json','final-checks.py','compare.py','session-comparison.json','core-comparison.json','ui-comparison.json','before-sha256.json','frozen-source.json','source-delta.json','source-delta.patch','final-source-verification.json','baseline-ui-supplement.json','write-evidence.py']:
 shutil.copy2(scratch/name,out/name)
shutil.copy2(scratch/'baseline/baseline.json',out/'baseline.json')
report={'date':'2026-09-12','checkpoint':'B6: native desktop manager and canonical project/session ownership; A/B/D partial','status':'accepted_source_checkpoint','commands_file':relative(out/'final-commands.json'),'results':results,'selected_total_passed':selected,'new_rust_contracts':8,'new_ui_contracts':5,'baseline_comparisons':comparisons,'compiler':subprocess.check_output(['rustc','--version'],text=True).strip(),'Cargo_lock_sha256':sha('product/Cargo.lock'),'source_manifest':{'path':relative(out/'source-sha256.json'),'sha256':sha(out/'source-sha256.json'),'count':len(frozen)},'source_delta':relative(out/'source-delta.json'),'binary_sha256':binaries,'preservation':{'path':relative(out/'preservation.json'),'all_unchanged':True,'entries':len(protected)},'native_visual':{'result':relative(visual_out/'result.json'),'inspection':relative(visual_out/'inspection.json'),'sha256':sha(visual_out/'result.json')},'clippy':'Exit0; same63 existing platform warnings. New lease warning corrected before final freeze.','no_frozen_expectation_changes':True,'new_dependencies':[],'production_installed':False,'real_inference':False,'limitations':['Private credentials/OAuth and reviewed read/write trust grants are unfinished.','Discover is explicitly unavailable until public registry and guided setup are implemented.','Supported input continuation, media presentation, child MCP delegation and full A-F conformance/budget acceptance remain.','Missing working folders receive no MCP runtime; normal folder recovery remains available.','Current conservative host-account local launch requires explicit consent and is not a filesystem sandbox.','Native visual fixture is not an installable production checkpoint; normal production build/install is next.']}
report['artifact_sha256']={relative(p):sha(p) for p in sorted(out.rglob('*')) if p.is_file()}
write(root/'implementation/evidence/MCP/desktop-integration.json',report)
write(scratch/'acceptance-ready.json',{'checkpoint':'B6','accepted_source_manifest':relative(out/'source-sha256.json'),'source_sha256':sha(out/'source-sha256.json'),'report':relative(root/'implementation/evidence/MCP/desktop-integration.json'),'selected_passes':selected,'full_mcp_complete':False,'production_installation_pending':True})
print(json.dumps({'accepted':'B6 source checkpoint','selected_passes':selected,'source_files':len(frozen),'preserved':len(protected)}))
