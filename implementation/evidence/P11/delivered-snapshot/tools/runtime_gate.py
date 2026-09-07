"""P10 acceptance executes native/reference behavior, faults, measurements and preservation."""
import datetime
from pathlib import Path
import re
import shutil
import tempfile
from common import ROOT, REFERENCE, GateError, read_json, write_json, sha256, run, verify_original, isolated_env
from runtime_differential import inputs
from tool_gate import environment, node_cases
from provider_gate import validate_rust_log

CAPABILITIES = {f'A{i:02}' for i in range(2,15)} | {'A17','A26','A48','A70'}
INVARIANTS = {f'I{i:02}' for i in [1,2,3,4,5,7,8,13,15,16,17,18,19,22,25]}
REQUIRED = CAPABILITIES | INVARIANTS | {f'L{i:02}' for i in range(1,9)} | {f'B{i:02}' for i in range(1,5)}
PACKAGES = ['ag1-core','ag1-storage','ag1-protocol','ag1-platform']

def validate_trace_map(value, passed):
    rows = value.get('rows',[])
    if value.get('version') != 1 or value.get('phase') != 'P10' or {r.get('id') for r in rows} != REQUIRED or len(rows) != len(REQUIRED):
        raise GateError('Incomplete or duplicate P10 requirement trace map')
    for row in rows:
        if not row.get('behavior'): raise GateError('Missing trace interpretation')
        for side in ['success','failure']:
            cases = row.get(side,[])
            if not cases: raise GateError('Missing '+side+' trace: '+row['id'])
            for case in cases:
                if (case.get('path'),case.get('test')) not in passed:
                    raise GateError('Unexecuted trace: '+row['id']+' '+str(case))
    return value

def package_cases(package, log, expected):
    # Cargo emits separate harness summaries. Ignore zero-test library/examples,
    # but every selected integration suite must actually execute with no skips.
    summaries = re.findall(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured; (\d+) filtered out;', log)
    if not summaries or any(any(int(v) for v in row[1:]) for row in summaries):
        raise GateError('Skipped or incomplete native regression')
    cases = set();found = set()
    chunks = re.split(r'\s+Running ',log)[1:]
    for chunk in chunks:
        first = chunk.splitlines()[0]
        suite = re.match(r'tests/([a-zA-Z0-9_]+)\.rs ',first)
        if not suite: continue
        name = suite[1]
        if name in found: raise GateError('Duplicate native suite')
        found.add(name)
        validate_rust_log(chunk,1)
        path = f'product/crates/{package}/tests/{name}.rs'
        cases.update((path,n) for n in re.findall(r'^test ([a-zA-Z_][a-zA-Z0-9_:]*) \.\.\. ok$',chunk,re.M))
    if found != set(expected): raise GateError(f'Missing integration suites for {package}: {set(expected)-found}')
    return cases

def validate_replay(report, before):
    from reference_oracles import input_fingerprints
    if (report.get('status') != 'passed' or report.get('phase_complete') is not False
        or report.get('input_fingerprints') != {'product':before,'reference':input_fingerprints()}
        or not report.get('commands') or any(r.get('exit_code') != 0 for r in report['commands'])):
        raise GateError('Missing, skipped or stale runtime differential')

def preservation():
    from phase_gate import verify_runtime_material
    lock = read_json(ROOT/'planning/evidence/llama-runtime-lock.json')
    if sha256(Path(lock['profile_file'])) != lock['profile_file_sha256']:
        raise GateError('Tuned llama profiles changed')
    material = read_json(ROOT/'implementation/evidence/P01/runtime-manifest.json')
    verify_runtime_material(material,read_json(ROOT/'.local/runtime/hash-cache.json'))
    checked = {}
    for model in lock['models'].values():
        for row in model['binary_and_adjacent_shared_libraries']:
            if sha256(Path(row['path'])) != row['sha256']: raise GateError('Tuned runtime binary/library changed')
            checked[row['path']] = row['sha256']
    return {'profile_sha256':lock['profile_file_sha256'],'binary_and_adjacent_libraries':checked,
            'weight_files':len(material['weights']),'loader_objects':len(material['loader_objects']),
            'model_actions':0,'profile_writes':0}

def execute_p10():
    from phase_gate import verify_dependencies, verify_artifacts
    from runtime_licenses import verify as licenses
    from runtime_measurement import measure
    evidence = ROOT/'implementation/evidence/P10';evidence.mkdir(parents=True,exist_ok=True)
    completion = evidence/'completion.json';commands=[]
    write_json(completion,{'phase':'P10','status':'running'})
    node,env = environment()
    def execute(name,argv,selected_env=env):
        print('P10: '+name,flush=True)
        log=evidence/(name+'.log');commands.append(run(argv,env=selected_env,log=log,timeout=1200));return log.read_text()
    try:
        verify_dependencies('P10');original=verify_original();preserved=preservation()
        execute('native-license-capture',['python3','tools/record_protocol_licenses.py'])
        write_json(evidence/'licenses.json',licenses())
        before=inputs()
        execute('format',['cargo','fmt','--manifest-path','product/Cargo.toml','--all','--','--check'])
        execute('included-fixture-format',['rustfmt','--edition','2024','--check','product/crates/ag1-platform/tests/support/session_fixture.rs','product/crates/ag1-platform/examples/runtime_probe/measure.rs'])
        execute('clippy',['cargo','clippy','--offline','--manifest-path','product/Cargo.toml',*[v for p in PACKAGES for v in ['-p',p]],'--all-targets','--','-D','warnings'])
        validator=execute('gate-validator',['python3','-m','unittest','discover','-s','tools/tests','-p','test_runtime*.py','-v'])
        if not re.search(r'\nRan [1-9]\d* tests in [^\n]+\n\nOK\n',validator): raise GateError('Empty gate validator selection')
        passed=set()
        for package in PACKAGES:
            log=execute(package+'-regression',['cargo','test','--offline','--manifest-path','product/Cargo.toml','-p',package,'--all-targets'])
            expected=[p.stem for p in (ROOT/f'product/crates/{package}/tests').glob('*.rs')]
            passed.update(package_cases(package,log,expected))
        helper_files = sorted((ROOT/'product/compat/extensions/test').glob('*.test.mjs'))
        if {p.name for p in helper_files} != {'event-stream.test.mjs','runtime.test.mjs','transport.test.mjs'}:
            raise GateError('Missing helper contract suite')
        for path in helper_files:
            relative=path.relative_to(ROOT).as_posix()
            log=execute('helper-'+path.stem,[str(node),'--test','--test-reporter=tap',relative])
            passed.update(node_cases(relative,log))
        execute('runtime-differential',['python3','tools/reference_oracles.py','--suite','runtime-differential','--offline'])
        replay=read_json(evidence/'oracle-runtime-differential.json');validate_replay(replay,before)
        coverage=validate_trace_map(read_json(ROOT/'fixtures/runtime/trace-map.json'),passed)
        write_json(evidence/'coverage.json',coverage)
        with tempfile.TemporaryDirectory(prefix='ag1-p10-launch-') as directory:
            isolated=isolated_env(Path(directory)/'env');isolated['PATH']=str(node.parent)+':'+isolated['PATH']
            isolated['NODE_OPTIONS']='--import='+str(ROOT/'tools/offline_guard.mjs')
            target=Path(directory)/'capture'
            execute('unchanged-launch-plans',[str(node),'tools/probes/capture_llama.mjs',str(REFERENCE),str(target)],isolated)
            for name in ['launch-plans.json','request-payloads.json']:
                if sha256(target/name) != sha256(ROOT/'fixtures/llama'/name): raise GateError('Pinned llama launch/request output changed')
        execute('measurement-build',['cargo','build','--offline','--release','--manifest-path','product/Cargo.toml','-p','ag1-platform','--example','runtime-probe'])
        performance=measure(ROOT/'product/target/release/examples/runtime-probe',evidence,node)
        commands.extend({k:r[k] for k in ['argv','exit_code','seconds','log']} for r in performance['runs'])
        licenses();verify_dependencies('P10');verify_original()
        if preserved != preservation() or before != inputs(): raise GateError('P10 inputs or preserved runtime changed during acceptance')
        write_json(evidence/'runtime-preservation.json',preserved)
        write_json(evidence/'passed-cases.json',[{'path':p,'test':t} for p,t in sorted(passed)])
        totals = performance['summary']['total_ns']
        (evidence/'REPORT.md').write_text(
            '# P10 native session runtime acceptance\n\n'
            f'The executed P10 gate passed {len(passed)} native/helper cases and replayed {len(replay["reference_probes"])} pinned source probes with {sum(len(v) for v in replay["contracts"].values())} Rust runtime contract entries. The crash-child entry is a harness for its parent process-crash scenario, not a second independent crash scenario. [completion.json](completion.json) retains exact commands, input hashes, dependency receipts and immutable delivered artifacts. [coverage.json](coverage.json) maps every required A/I/L row and B01–B04 to passing success/failure assertions.\n\n'
            'The Rust SessionRuntime/NativeSession and durable RuntimeRegistry/RuntimeCommands compose current-state request admission, acknowledged tool/context/provider work, queue controls, retries/compaction, restart inspection, mode-selected completion/planning and isolated subagents. Optional admitted extension tools/callbacks use the native owner. There is no hidden TS AgentSession in the product. The normal TypeScript desktop remains production authority; no desktop or real model was launched.\n\n'
            'Required checks include Rust formatting, strict four-crate Clippy, all four crates’ test targets, all helper tests, gate rejection tests, the fresh runtime differential, source/native provenance and dependency notices, nine unchanged llama launch plans/six payload captures, tuned runtime material verification and three alternating TS/Rust measurement runs. Failed or skipped mandatory selections and stale inputs cannot produce a passed completion.\n\n'
            f'Matched fake workload total p95: Rust {totals["rust"]["p95_ns"]/1e6:.3f} ms, TS {totals["typescript"]["p95_ns"]/1e6:.3f} ms. [performance.json](performance.json) reports preparation/tool/context/follow-up intervals and all noise samples. SQL FULL durability and original-input/output custody differ from TS JSONL; these are host cost observations, not an equivalent-durability or pure-core regression claim. Each run retains all 130 journal entries and projects two messages after 32 cumulative resume/compaction cycles. Heap/RSS are finite-workload observations; physical history intentionally grows. No live model or whole-desktop speedup is claimed.\n\n'
            'All original source pins, three tuned llama.cpp backends, adjacent/loader libraries, weights and per-model profile bytes were reverified. [runtime-preservation.json](runtime-preservation.json) records the contribution to L01–L08. No rebuild, stock replacement, tuning, original profile write or process ownership adoption occurred.\n\n'
            'Compatibility boundaries: callback mutation/provider/process/UI bridges use explicit unavailable responses in this native binding; dynamic registration requires a new admitted host. Payload callbacks cannot replace model/tools/stream or private endpoint/credentials. Tree navigation rejects unresolved tool boundaries. Verification is native-receipt-bound and request-local; later mutations/Unknown effects invalidate prior checks. P09 core resource precedence/expansion remains available; P12 owns slash/skill/template client routing, SDK/CLI/UI facades and export rendering. The source branch-summary probe omits only undefined fields at its persisted JSON boundary. See the delivered SESSION.md and scope.json for exact APIs and decisions.\n\n'
            'P11 receives an executable Rust runtime for isolated target integration; P12 can bind clients to its commands. P13 authority cutover and P14 live per-model, Windows/macOS, packaging and 8h/24h release tests remain explicit later gates. This phase does not certify them.\n\n'
            'Rollback: stop only owned test resources, retain uncertain outcomes and inspection evidence, and restore the P09 commit 7c3fe87d7943c3f214dba81296c33e1b9a53e05f. Original Cruzible, desktop authority, llama installations, configuration and weights remain unchanged. The user requested a P10 commit and stop; do not start P11.\n')

        # Immutable delivered inputs allow later phases to evolve without
        # invalidating dependency acceptance evidence.
        for relative in before:
            target=evidence/'delivered-snapshot'/relative;target.parent.mkdir(parents=True,exist_ok=True);shutil.copyfile(ROOT/relative,target)
        artifacts=[{'path':p.relative_to(ROOT).as_posix(),'sha256':sha256(p)} for p in sorted(evidence.rglob('*')) if p.is_file() and p!=completion]
        verify_artifacts(ROOT,artifacts)
        limitations=read_json(ROOT/'fixtures/runtime/scope.json')
        write_json(completion,{'phase':'P10','status':'passed','completed_at':datetime.datetime.now(datetime.timezone.utc).isoformat(),
            'input_fingerprints':{'product':before,'original':original,'reference':replay['input_fingerprints']['reference']},
            'dependencies':['P08','P09'],'artifacts':artifacts,'commands':commands,
            'coverage':{'requirements':sorted(REQUIRED),'passing_native_and_helper_cases':len(passed),'runtime_contracts':sum(len(v) for v in replay['contracts'].values()),'reference_probes':len(replay['reference_probes'])},
            'authority':{'before':'TypeScript desktop/session and externally owned tuned llama.cpp runtimes','after':'Unchanged; native P10 runtime executes only in disposable fixtures'},
            'pending_gates':limitations['later_gates'],'scope':limitations})
        print(f'P10 passed: {len(passed)} native/helper cases; 38 source probes; runtime preserved.',flush=True)
    except Exception as error:
        write_json(completion,{'phase':'P10','status':'failed','commands':commands,'error':str(error),'authority':'Original sources, desktop and tuned runtimes unchanged'})
        raise
