"""P12 acceptance: native clients, matched WebKit measurements and preservation."""
from __future__ import annotations
import datetime
import html
import json
from pathlib import Path
import re
import shutil
import tempfile
from common import ROOT, REFERENCE, PIN, GateError, read_json, write_json, sha256, run, verify_original
from tool_gate import environment, node_cases
from runtime_gate import preservation, package_cases
from phase_gate import verify_dependencies, verify_artifacts
from project_gate import verify_tooling
from provider_gate import validate_rust_log

PACKAGES=['ag1-core','ag1-storage','ag1-protocol','ag1-platform','ag1-cli']
REQUIRED={'A01','A26',*[f'A{i:02}' for i in range(72,76)],*[f'A{i:02}' for i in range(79,86)],*[f'I{i:02}' for i in [1,20,22,23,26,27,28,30]],*[f'L{i:02}' for i in range(1,9)]}
VISUAL={'1000_message_scroll','failed_save_retains_dirty_buffer','native_plan_review','model_conflict_no_retune','native_resource_approval','provider_failure_visible','display_latency','frozen_renderer_resnapshot','native_retry_visible','native_compaction','native_new_session_empty'}
SOURCES=['LICENSE','packages/coding-agent/package.json','packages/coding-agent/src/index.ts','packages/coding-agent/src/cli/args.ts','packages/coding-agent/src/modes/interactive/interactive-mode.ts','packages/coding-agent/src/modes/print-mode.ts','packages/coding-agent/src/modes/rpc/rpc-mode.ts','packages/coding-agent/src/core/workflow-engine.ts','packages/coding-agent/src/core/export-html/index.ts','packages/coding-agent/src/core/mcp-client.ts','packages/coding-agent/src/core/auton/index.ts',*[f'packages/{name}/package.json' for name in ['web-ui','mom','pods']]]

def inputs():
    paths=[]
    for folder in ['product/crates','product/apps/desktop/src','product/apps/desktop/src-tauri','product/clients','product/licenses','fixtures/clients']:
        paths.extend(p for p in (ROOT/folder).rglob('*') if p.is_file() and not {'target','node_modules','dist','gen','__pycache__'}&set(p.relative_to(ROOT/folder).parts))
    paths.extend(ROOT/p for p in ['product/Cargo.toml','product/Cargo.lock','product/LICENSE','product/apps/desktop/package.json','product/apps/desktop/package-lock.json','product/apps/desktop/vite.config.ts','product/apps/desktop/tsconfig.json','tools/client_gate.py','tools/client_visual.py','tools/tests/test_client_gate.py','tools/probes/desktop_visual.py','tools/audit_desktop_bundle.mjs','tools/phase_gate.py','tools/common.py','tools/runtime_gate.py','tools/project_gate.py','tools/tool_gate.py','tools/provider_gate.py','phases/P12-ui-and-client-parity.md','implementation/P12-HANDOFF.md'])
    return {p.relative_to(ROOT).as_posix():sha256(p) for p in sorted(set(paths))}

def validate_visual(value,root=ROOT):
    if value.get('status')!='passed' or value.get('viewport')!={'width':1440,'height':900,'dpr':1}:raise GateError('Unmatched or incomplete native visual run')
    cases=value.get('cases',{})
    if not VISUAL<=cases.keys() or any(cases[c].get('passed') is not True for c in VISUAL):raise GateError('Missing or failed native UI scenario')
    latency=value.get('latency',{})
    if latency.get('samples',0)<95 or not 0<=latency.get('p95_ms',999)<50:raise GateError('Native streaming display p95 must be below 50 ms')
    freeze=cases['frozen_renderer_resnapshot']
    if freeze.get('frozen_ms',0)<30000 or not 0<=freeze.get('provider_completed_ms',99999)<30000:raise GateError('Renderer freeze did not prove independent native completion')
    if not 0<cases['1000_message_scroll'].get('rendered_articles',1000)<100:raise GateError('Conversation is not virtualized')
    screenshots=value.get('screenshots',[])
    names=[s.get('name') for s in screenshots]
    if len(set(names))!=len(names) or not {'home-light','home-dark','chat-light','chat-dark','dialog-light','files-dirty-light','files-conflict-light','plans-light','context-light','activity-light','runtime-light','terminal-light','approval-light','models-conflict-light','resnapshot-light'}<=set(names):raise GateError('Missing matched native screenshots')
    for row in screenshots:
        path=(root/row['path']).resolve()
        if not path.is_relative_to(root.resolve()) or not path.is_file():raise GateError('Missing visual artifact')
        data=path.read_bytes()
        if data[:8]!=b'\x89PNG\r\n\x1a\n' or len(data)<100 or data[16:24]!=bytes.fromhex('000005a000000384'):raise GateError('Invalid screenshot dimensions or format')
    return value

def validate_dispositions(value):
    rows=value.get('rows',[])
    if value.get('phase')!='P12' or len(rows)!=85 or {r.get('id') for r in rows}!={f'A{i:02}' for i in range(1,86)}:raise GateError('Missing or duplicated A01–A85 disposition')
    for row in rows:
        if not all(row.get(k) for k in ['capability','disposition','client_surface','detail']):raise GateError('Empty client disposition')
    statuses={r['id']:r['disposition'] for r in rows}
    for key,expected in {'A79':'text-steering-reference-dag','A80':'reference-unintegrated','A81':'removed-reference','A82':'reference-client','A83':'reference-client','A84':'reference-client','A85':'explicit-unsupported'}.items():
        if statuses[key]!=expected:raise GateError('Adjacent/dead client silently advertised as migrated: '+key)
    return value

def validate_coverage(value,passed):
    rows=value.get('rows',[])
    if value.get('phase')!='P12' or len(rows)!=len(REQUIRED) or {r.get('id') for r in rows}!=REQUIRED:raise GateError('Incomplete P12 requirement coverage')
    for row in rows:
        for side in ['success','failure']:
            if not row.get(side):raise GateError('Missing client trace '+row['id'])
            for case in row[side]:
                if (case.get('path'),case.get('test')) not in passed:raise GateError('Unexecuted client trace '+str(case))
    return value

def visual_comparison(evidence,report):
    records=[];rows=[]
    for state in ['home-light','home-dark','chat-light','chat-dark','dialog-light','files-dirty-light']:
        oldname='files-dirty-light' if state=='files-dirty-light' else state
        old=ROOT/'implementation/evidence/P04/screenshots'/('ag1-'+oldname+'.png')
        if not old.exists() and state=='files-dirty-light':old=ROOT/'implementation/evidence/P04/screenshots/ag1-files-light.png'
        new=next(ROOT/r['path'] for r in report['screenshots'] if r['name']==state)
        if not old.is_file():raise GateError('Missing approved P04 comparison '+state)
        records.append({'state':state,'P04':{'path':old.relative_to(ROOT).as_posix(),'sha256':sha256(old)},'P12':{'path':new.relative_to(ROOT).as_posix(),'sha256':sha256(new)}})
        rows.append(f'<h2>{html.escape(state)}</h2><div><figure><figcaption>P04</figcaption><img src="../P04/screenshots/{old.name}"></figure><figure><figcaption>P12</figcaption><img src="screenshots/{new.name}"></figure></div>')
    (evidence/'visual-comparison.html').write_text('<!doctype html><meta charset="utf-8"><title>P12 native comparison</title><style>body{font:16px system-ui;background:#eee;color:#111}div{display:flex}figure{margin:8px;width:48%}img{width:100%}</style><h1>P04 and P12 at 1440 × 900, DPR 1</h1><p>Same packaged Geist fonts and local light/dark themes. P12 adds native status and typed messages. Fixture content differs; this is a layout review, not pixel identity or a live-model benchmark.</p>'+''.join(rows))
    write_json(evidence/'visual-comparison.json',records)

def execute_p12():
    evidence=ROOT/'implementation/evidence/P12';evidence.mkdir(parents=True,exist_ok=True);completion=evidence/'completion.json';commands=[]
    write_json(completion,{'phase':'P12','status':'running'});node,env=environment()
    def execute(name,argv,selected_env=None):
        print('P12: '+name,flush=True);log=evidence/(name+'.log');commands.append(run(argv,env=selected_env or env,log=log,timeout=1800));return log.read_text()
    try:
        verify_dependencies('P12');original=verify_original();preserved=preservation();tooling=verify_tooling()
        execute('native-license-capture',['python3','tools/record_protocol_licenses.py'])
        execute('sdk-build',[str(node),'product/clients/sdk/build.mjs'])
        before=inputs()
        execute('format',['cargo','fmt','--manifest-path','product/Cargo.toml','--all','--','--check'])
        execute('clippy',['cargo','clippy','--offline','--manifest-path','product/Cargo.toml',*[v for p in PACKAGES for v in ['-p',p]],'--all-targets','--','-D','warnings'])
        execute('desktop-clippy',['cargo','clippy','--offline','--manifest-path','product/Cargo.toml','-p','ag1-desktop','--all-targets','--features','visual-test','--','-D','warnings'])
        execute('protocol-check',['cargo','run','--offline','--manifest-path','product/Cargo.toml','-p','ag1-protocol','--bin','protocol-codegen','--','--check'])
        validator=execute('gate-tests',['python3','-m','unittest','discover','-s','tools/tests','-p','test_client_gate.py','-v'])
        if not re.search(r'\nRan [1-9]\d* tests in [^\n]+\n\nOK\n',validator):raise GateError('Empty gate validator selection')
        execute('frontend-typecheck',['npm','--prefix','product/apps/desktop','run','typecheck'])
        execute('frontend-tests',['npm','--prefix','product/apps/desktop','test','--','--run','--reporter=json','--outputFile='+str(evidence/'frontend-tests.json')])
        from reference_oracles import validate_vitest
        frontend=read_json(evidence/'frontend-tests.json');validate_vitest(frontend,expected=frontend.get('numTotalTests',0))
        if frontend.get('numTotalTests',0)<14:raise GateError('Incomplete frontend contracts')
        execute('frontend-build',['npm','--prefix','product/apps/desktop','run','build'])
        execute('renderer-audit',[str(node),'tools/audit_desktop_bundle.mjs'],{**env,'AG1_BUNDLE_AUDIT_OUTPUT':str(evidence/'renderer-bundle-audit.json')})
        audit=read_json(evidence/'renderer-bundle-audit.json')
        if audit.get('status')!='audited' or len(audit.get('licenses',[]))<30:raise GateError('Incomplete frontend license closure')
        for name in ['Editor-','TerminalPane-']:
            if not any(Path(a['path']).name.startswith(name) and a['path'].endswith('.js') for a in audit['assets']):raise GateError('Missing lazy client module '+name)
        execute('native-build',['cargo','build','--offline','--manifest-path','product/Cargo.toml','-p','ag1-desktop','-p','ag1-platform','-p','ag1-cli','--bins','--examples','--features','ag1-desktop/visual-test'])
        passed=set()
        for package in PACKAGES:
            log=execute(package+'-contracts',['cargo','test','--offline','--manifest-path','product/Cargo.toml','-p',package,'--all-targets'])
            folder=ROOT/('product/clients/cli/tests' if package=='ag1-cli' else f'product/crates/{package}/tests')
            cases=package_cases(package,log,[p.stem for p in folder.glob('*.rs')])
            passed.update((p.replace('product/crates/ag1-cli/','product/clients/cli/'),n) for p,n in cases)
        log=execute('desktop-contracts',['cargo','test','--offline','--manifest-path','product/Cargo.toml','-p','ag1-desktop','--test','bridge_contract','--features','visual-test']);validate_rust_log(log,3)
        sdk='product/clients/sdk/contract.test.mjs';passed.update(node_cases(sdk,execute('sdk-contracts',[str(node),'--test','--test-reporter=tap',sdk])))
        reference='fixtures/clients/reference-contract.test.mjs';passed.update(node_cases(reference,execute('reference-client-contracts',[str(node),'--test','--test-reporter=tap',reference])))
        for suite in frontend['testResults']:
            path=Path(suite['name']).relative_to(ROOT).as_posix()
            passed.update((path,c['fullName']) for c in suite['assertionResults'] if c['status']=='passed')
        with tempfile.TemporaryDirectory(prefix='p12-preview-',dir=ROOT/'.local') as directory:
            execute('native-preview-denial',[str(ROOT/'product/target/debug/examples/project-preview-probe'),directory]);preview=read_json(Path(directory)/'interactive-webkit.json')
            if preview.get('status')!='passed' or preview.get('observation',{}).get('denied') is not True or preview.get('blocked_navigation') is not True or preview.get('closed_preview') is not True:raise GateError('Native preview privilege denial failed')
            shutil.copyfile(Path(directory)/'interactive-webkit.json',evidence/'interactive-webkit.json');shutil.copyfile(Path(directory)/'interactive-webkit.png',evidence/'interactive-webkit.png')
        execute('native-visual',['python3','tools/client_visual.py','--output',str(evidence/'native-visual.json')])
        visual=validate_visual(read_json(evidence/'native-visual.json'));passed.update(('tools/client_visual.py',name) for name in VISUAL);screens=evidence/'screenshots';screens.mkdir(exist_ok=True)
        for row in visual['screenshots']:
            source=ROOT/row['path'];target=screens/source.name;shutil.copyfile(source,target);row['path']=target.relative_to(ROOT).as_posix()
        write_json(evidence/'native-visual.json',visual);visual_comparison(evidence,visual)
        dispositions=validate_dispositions(read_json(ROOT/'fixtures/clients/dispositions.json'));coverage=validate_coverage(read_json(ROOT/'fixtures/clients/trace-map.json'),passed)
        write_json(evidence/'coverage.json',coverage);write_json(evidence/'dispositions.json',dispositions);write_json(evidence/'passed-cases.json',[{'path':p,'test':n} for p,n in sorted(passed)])
        sources={p:sha256(REFERENCE/p) for p in SOURCES}
        write_json(evidence/'provenance.json',{'source_commit':PIN,'source_files':sources,'notice':{'path':'product/LICENSE','sha256':sha256(ROOT/'product/LICENSE')},'implementation':'Original native adapter composition and React surfaces; source action/workflow semantics retain AG1 MIT attribution. No additional Dyad source or third-party raster assets copied. P04 approved primitives/fonts and emitted closure audited separately.','destinations':before})
        verify_dependencies('P12');verify_original()
        if before!=inputs() or preserved!=preservation() or tooling!=verify_tooling():raise GateError('Source, tools or preserved runtime changed during acceptance')
        write_json(evidence/'runtime-preservation.json',preserved)
        for relative in before:
            target=evidence/'delivered-snapshot'/relative;target.parent.mkdir(parents=True,exist_ok=True);shutil.copyfile(ROOT/relative,target)
        for name in ['REPORT.md','../../P13-HANDOFF.md']:
            if not (evidence/name).is_file():raise GateError('Missing final report/handoff '+name)
        artifacts=[{'path':p.relative_to(ROOT).as_posix(),'sha256':sha256(p)} for p in sorted(evidence.rglob('*')) if p.is_file() and p!=completion]
        artifacts.append({'path':'implementation/P13-HANDOFF.md','sha256':sha256(ROOT/'implementation/P13-HANDOFF.md')});verify_artifacts(ROOT,artifacts)
        write_json(completion,{'phase':'P12','status':'passed','completed_at':datetime.datetime.now(datetime.timezone.utc).isoformat(),'dependencies':['P11'],'input_fingerprints':{'product':before,'original':original},'commands':commands,'artifacts':artifacts,'coverage':{'requirements':sorted(REQUIRED),'capability_dispositions':85,'native_and_sdk_cases':len(passed),'frontend_tests':frontend['numTotalTests'],'native_visual':visual['cases']},'authority':{'before':'Normal TypeScript desktop; externally owned tuned llama.cpp runtimes','after':'Unchanged; Rust CLI/JSONL/SDK and desktop usable in explicitly isolated workspaces'},'pending_gates':['P13 explicit desktop authority and live endpoint ownership transfer','P14 Windows/macOS, final packaging, live quality and matched model performance, 8h/24h workloads','Reference-only/dead/removed interfaces retain their explicit compatibility dispositions']})
        print(f'P12 passed: {len(passed)} native/SDK cases, {frontend["numTotalTests"]} frontend tests; native WebKit p95 {visual["latency"]["p95_ms"]} ms; tuned runtime preserved.',flush=True)
    except Exception as error:
        write_json(completion,{'phase':'P12','status':'failed','commands':commands,'error':str(error),'authority':'Original sources, normal TS desktop and tuned runtime preserved'});raise
