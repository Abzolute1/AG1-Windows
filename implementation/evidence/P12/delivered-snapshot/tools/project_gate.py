"""P11 executes project runtimes, native WebKit, protocol, faults and preservation."""
from __future__ import annotations
import datetime
import json
from pathlib import Path
import re
import shutil
import tempfile
from common import ROOT, REFERENCE, GateError, read_json, write_json, sha256, run, verify_original, isolated_env
from tool_gate import environment
from runtime_gate import preservation, package_cases
from phase_gate import verify_dependencies, verify_artifacts

PACKAGES=['ag1-core','ag1-storage','ag1-protocol','ag1-platform']
REQUIRED={'A27','A45','A67','A69','A78','I07','I21','I25','I26','I27',*[f'L{i:02}' for i in range(1,9)]}
MEASUREMENTS=['rust-install','rust-build','rust-run','web-install','web-build','web-ready','web-stopped','tauri-rendered','tauri-crash','tauri-dev-install','tauri-dev-build','tauri-dev-run']

def inputs():
    paths=[]
    for folder in ['product/crates','product/apps/desktop/src-tauri/src','product/apps/desktop/src-tauri/examples','product/compat/project-browser','fixtures/projects']:
        paths.extend(p for p in (ROOT/folder).rglob('*') if p.is_file())
    paths.extend(ROOT/p for p in ['product/Cargo.toml','product/Cargo.lock','product/apps/desktop/src-tauri/Cargo.toml','product/apps/desktop/src-tauri/tauri.conf.json','product/apps/desktop/src-tauri/project-fixture.conf.json','implementation/P12-HANDOFF.md','implementation/evidence/P11/REPORT.md','tools/project_gate.py','tools/tests/test_project_gate.py','tools/phase_gate.py','tools/common.py','tools/runtime_gate.py','tools/provider_gate.py','tools/tool_gate.py','phases/P11-project-and-native-runtimes.md'])
    return {p.relative_to(ROOT).as_posix():sha256(p) for p in sorted(set(paths)) if '__pycache__' not in p.parts}

def verify_tooling():
    lock=read_json(ROOT/'fixtures/projects/tooling-lock.json')
    for name,row in lock.items():
        path=ROOT/row['path']
        if not path.is_file() or sha256(path)!=row['sha256']:raise GateError(f'Missing/changed {name}; restore the pinned optional fixture tool from fixtures/projects/tooling-lock.json, never substitute a model runtime')
    for row in read_json(ROOT/'fixtures/projects/provenance.json')['references']:
        if sha256(ROOT/row['source'])!=row['sha256']:raise GateError('Changed protocol/check provenance '+row['source'])
    return lock

def validate_measurements(directory:Path):
    records={name:read_json(directory/(name+'.json')) for name in MEASUREMENTS}
    for name,value in records.items():
        if not value.get('commands') or not value.get('logs') or not value.get('source_revision') or value.get('elapsed_ms',-1)<0:
            raise GateError('Incomplete executed operation '+name)
        if name == 'tauri-dev-run' and (value.get('lifecycle')!='exited' or value.get('exits',[])[-1:]!=[0]):raise GateError('Missing actual Tauri dev native exit')
        if name not in ['web-ready','web-stopped','tauri-crash','tauri-dev-run'] and (value.get('lifecycle')!='exited' or any(code!=0 for code in value.get('exits',[]))):raise GateError('Missing native/compiler success '+name)
    ready=records['web-ready'];stop=records['web-stopped']
    if ready.get('lifecycle')!='ready' or ready.get('ready_ms') is None or ready.get('first_log_ms') is None or ready.get('target_rss_bytes',0)<=0 or stop.get('stop_ms') is None:
        raise GateError('Missing run-ready/log-availability/stop/memory samples')
    if records['tauri-crash']['lifecycle']!='failed':raise GateError('Native crash was not observed')
    browser=read_json(directory/'browser.json')
    if browser.get('browser_rss_bytes',0)<=0 or browser.get('total_rss_bytes')!=browser.get('browser_rss_bytes',0)+browser.get('target_rss_bytes',0):raise GateError('Browser memory excluded from total')
    if browser['observation']['privileged']!={'node':False,'tauri':False}:raise GateError('Generated browser inherited privileges')
    if not any('deliberate frontend runtime failure' in e for e in browser['observation'].get('errors',[])):raise GateError('Frontend runtime failure was not exercised')
    webkit=read_json(directory/'interactive-webkit.json')
    if webkit.get('status')!='passed' or webkit['observation'].get('denied') is not True or webkit['observation'].get('count')!='1' or webkit.get('blocked_navigation') is not True or webkit.get('closed_preview') is not True:raise GateError('Interactive native preview proof incomplete')
    for name in ['browser.png','tauri.png','tauri-crash.png','interactive-webkit.png']:
        path=directory/name
        if not path.is_file() or path.stat().st_size<=100 or path.read_bytes()[:8]!=b'\x89PNG\r\n\x1a\n':raise GateError('Missing actual visual artifact '+name)
    return {'operations':records,'browser':browser,'webkit':webkit,'scope':'Development Linux fixtures; first_log_ms is retained log availability at the target port, not final P12 renderer latency. Compiler/install durations are separate operations. RSS sums are conservative sums of observed group peaks, including P03 helpers. Private Xvfb is separate display_rss_bytes. No live inference, GPU or whole-desktop performance claim.'}

def validate_coverage(value,passed):
    rows=value.get('rows',[])
    if value.get('phase')!='P11' or len(rows)!=len(REQUIRED) or {r.get('id') for r in rows}!=REQUIRED:raise GateError('P11 requirement coverage incomplete/duplicated')
    for row in rows:
        for side in ['success','failure']:
            if not row.get(side):raise GateError('Missing requirement trace '+row['id'])
            for case in row[side]:
                if (case.get('path'),case.get('test')) not in passed:raise GateError('Unexecuted P11 trace '+str(case))
    return value

def execute_p11():
    evidence=ROOT/'implementation/evidence/P11';evidence.mkdir(parents=True,exist_ok=True)
    completion=evidence/'completion.json';commands=[];write_json(completion,{'phase':'P11','status':'running'})
    node,env=environment()
    def execute(name,argv,cwd=ROOT,selected_env=None):
        print('P11: '+name,flush=True);log=evidence/(name+'.log')
        commands.append(run(argv,cwd=cwd,env=selected_env or env,log=log,timeout=1200));return log.read_text()
    try:
        verify_dependencies('P11');original=verify_original();preserved=preservation();tooling=verify_tooling();before=inputs()
        execute('format',['cargo','fmt','--manifest-path','product/Cargo.toml','--all','--','--check'])
        execute('clippy',['cargo','clippy','--offline','--manifest-path','product/Cargo.toml',*[v for p in PACKAGES for v in ['-p',p]],'--all-targets','--','-D','warnings'])
        execute('desktop-clippy',['cargo','clippy','--offline','--manifest-path','product/Cargo.toml','-p','ag1-desktop','--all-targets','--features','visual-test','--','-D','warnings'])
        execute('browser-syntax',[str(node),'--check','product/compat/project-browser/driver.mjs'])
        validator=execute('gate-tests',['python3','-m','unittest','discover','-s','tools/tests','-p','test_project_gate.py','-v'])
        if not re.search(r'\nRan [1-9]\d* tests in [^\n]+\n\nOK\n',validator):raise GateError('Empty gate validator selection')
        execute('native-fixture-build',['cargo','build','--offline','--manifest-path','product/Cargo.toml','-p','ag1-desktop','--examples','--features','visual-test'])
        passed=set()
        with tempfile.TemporaryDirectory(prefix='p11-evidence-',dir=ROOT/'.local') as directory:
            measurement=Path(directory);test_env={**env,'AG1_P11_EVIDENCE':str(measurement)}
            for package in PACKAGES:
                log=execute(package+'-contracts',['cargo','test','--offline','--manifest-path','product/Cargo.toml','-p',package,'--all-targets'],selected_env=test_env)
                expected=[p.stem for p in (ROOT/f'product/crates/{package}/tests').glob('*.rs')]
                passed.update(package_cases(package,log,expected))
            execute('interactive-native-preview',[str(ROOT/'product/target/debug/examples/project-preview-probe'),str(measurement)])
            measurements=validate_measurements(measurement)
            target=evidence/'measurements';target.mkdir(exist_ok=True)
            for p in measurement.iterdir():
                if p.is_file():shutil.copyfile(p,target/p.name)
        write_json(evidence/'performance.json',measurements)
        # Unchanged protocol/check consumers remain executable reference evidence.
        with tempfile.TemporaryDirectory(prefix='p11-reference-',dir=ROOT/'.local') as directory:
            ref_env=isolated_env(Path(directory));ref_env['PATH']=str(node.parent)+':'+ref_env['PATH'];ref_env['NODE_OPTIONS']='--import='+str(ROOT/'tools/offline_guard.mjs')
            for package,files in [('godot-agent-runtime',sorted((REFERENCE/'packages/godot-agent-runtime/test').glob('*.test.ts'))),('coding-agent',[REFERENCE/'packages/coding-agent/test'/p for p in ['check-detect.test.ts','check-parse.test.ts','check-tool.test.ts']])]:
                report=evidence/(package+'-reference.json')
                execute(package+'-reference',[str(node),str(REFERENCE/'node_modules/vitest/vitest.mjs'),'run',*[str(p.relative_to(REFERENCE/'packages'/package)) for p in files],'--reporter=json','--outputFile='+str(report)],cwd=REFERENCE/'packages'/package,selected_env=ref_env)
                from reference_oracles import validate_vitest
                value=read_json(report);validate_vitest(value,expected=value.get('numTotalTests',0))
                if value.get('numTotalTests',0)<=0:raise GateError('Empty pinned reference selection')
                if {str(Path(r['name']).resolve()) for r in value.get('testResults',[])}!={str(p.resolve()) for p in files}:raise GateError('Missing pinned reference test file')
        coverage=validate_coverage(read_json(ROOT/'fixtures/projects/trace-map.json'),passed)
        write_json(evidence/'coverage.json',coverage);write_json(evidence/'passed-cases.json',[{'path':p,'test':n} for p,n in sorted(passed)])
        verify_dependencies('P11');verify_original()
        if before!=inputs() or preserved!=preservation() or tooling!=verify_tooling():raise GateError('P11 inputs or preserved runtime changed during acceptance')
        write_json(evidence/'runtime-preservation.json',preserved);write_json(evidence/'tooling.json',tooling)
        for relative in before:
            target=evidence/'delivered-snapshot'/relative;target.parent.mkdir(parents=True,exist_ok=True);shutil.copyfile(ROOT/relative,target)
        required=['REPORT.md','../../P12-HANDOFF.md']
        for p in required:
            if not (evidence/p).is_file():raise GateError('Missing final report/handoff '+p)
        artifacts=[{'path':p.relative_to(ROOT).as_posix(),'sha256':sha256(p)} for p in sorted(evidence.rglob('*')) if p.is_file() and p!=completion]
        verify_artifacts(ROOT,artifacts)
        write_json(completion,{'phase':'P11','status':'passed','completed_at':datetime.datetime.now(datetime.timezone.utc).isoformat(),'input_fingerprints':{'product':before,'original':original},'dependencies':['P04','P10'],'artifacts':artifacts,'commands':commands,'coverage':{'requirements':sorted(REQUIRED),'native_cases':len(passed)},'authority':{'before':'TypeScript desktop and existing tuned llama.cpp owners','after':'Unchanged; native project targets execute only in isolated fixtures'},'pending_gates':['P12 client/UI action routing, final renderer log latency and extension facade','P13 desktop authority cutover','P14 Windows/macOS runners, packaging, live per-model and 8h/24h workloads','Native window embedding is unavailable; no embedding claim is made.']})
        print('P11 passed: '+str(len(passed))+' native cases; real browser/WebKit/native evidence; runtime preserved.',flush=True)
    except Exception as error:
        write_json(completion,{'phase':'P11','status':'failed','commands':commands,'error':str(error),'authority':'Original sources, desktop and tuned runtimes unchanged'})
        raise
