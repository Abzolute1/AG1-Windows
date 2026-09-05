"""P03 gate: real services on disposable fixtures, never user model services."""
import datetime
import json
import os
from pathlib import Path
import platform
import re
import shutil

from common import ROOT, REFERENCE, GateError, read_json, write_json, sha256, run, verify_original
from protocol_gate import validate_rust_log


def inputs():
    files = [p for p in (ROOT/'product').rglob('*') if p.is_file() and 'target' not in p.relative_to(ROOT/'product').parts]
    files += [p for p in (ROOT/'fixtures/protocol').rglob('*') if p.is_file()]
    files += [p for p in (ROOT/'tools').rglob('*') if p.is_file() and p.suffix in ['.py','.mjs'] and '__pycache__' not in p.parts]
    files += [ROOT/f'implementation/evidence/{phase}/completion.json' for phase in ['P01','P02']]
    files += [ROOT/'phases/P03-platform-services.md']
    return {str(p.relative_to(ROOT)):sha256(p) for p in sorted(files)}


def owned_fixture_processes():
    # Observation only; no process found by this scan can be adopted or terminated.
    executables = {str((ROOT/'product/target/debug'/name).resolve()) for name in ['ag1-process-host','ag1-platform-fixture']}
    found = []
    for entry in Path('/proc').iterdir():
        if not entry.name.isdecimal():continue
        try:
            if os.readlink(entry/'exe') in executables:found.append(int(entry.name))
        except OSError:pass
    return sorted(found)


def validate_platform_log(log):
    passed = validate_rust_log(log,minimum=26)
    measurements = [json.loads(line) for line in re.findall(r'^P03_MEASUREMENT (.+)$',log,flags=re.M)]
    if sorted(row['files'] for row in measurements)!=[120,900]:raise GateError('Missing small/large measurements')
    for row in measurements:
        if set(row['samples_ns'])!={'status_diff','index','search','read','write'}:raise GateError('Missing platform measurement')
        if any(len(samples)!=5 or any(n<=0 for n in samples) for samples in row['samples_ns'].values()):raise GateError('Incomplete latency samples')
    logs = [json.loads(line) for line in re.findall(r'^P03_LOG (.+)$',log,flags=re.M)]
    if len(logs)!=1 or logs[0]['memory_bytes']>65536 or logs[0]['disk_bytes']>8*1024*1024 or logs[0]['seen']<=8*1024*1024:
        raise GateError('Missing/overflowing log bound experiment')
    return passed,measurements,logs[0]


def execute_p03():
    from phase_gate import verify_artifacts,verify_dependencies,verify_runtime_material
    evidence=ROOT/'implementation/evidence/P03';evidence.mkdir(parents=True,exist_ok=True)
    completion=evidence/'completion.json';commands=[]
    write_json(completion,{'phase':'P03','status':'running'})
    try:
        if platform.system()!='Linux':raise GateError('P03 native services require Linux; other platforms pending')
        verify_dependencies('P03');original=verify_original()
        runtime=read_json(ROOT/'implementation/evidence/P01/runtime-manifest.json');cache=read_json(ROOT/'.local/runtime/hash-cache.json')
        verify_runtime_material(runtime,cache)
        lock=read_json(ROOT/'planning/evidence/llama-runtime-lock.json')
        if sha256(Path(lock['profile_file']))!=lock['profile_file_sha256']:raise GateError('Original runtime profile changed')
        if owned_fixture_processes():raise GateError('Existing fixture helper observed; refuse to adopt or stop it')
        before=inputs();cargo=['cargo','--offline'];manifest=['--manifest-path','product/Cargo.toml','--locked']
        checks=[
            ('gate-tests',['python3','-m','unittest','discover','-s','tools/tests','-v']),
            ('format',['cargo','fmt','--manifest-path','product/Cargo.toml','--all','--check']),
            ('clippy',cargo+['clippy']+manifest+['--workspace','--all-targets','--','-D','warnings']),
            ('platform-tests',cargo+['test']+manifest+['-p','ag1-platform','--test','platform_contract','--','--nocapture']),
            ('protocol-tests',cargo+['test']+manifest+['-p','ag1-protocol','--test','protocol_contract','--','--nocapture']),
            ('generated-drift',cargo+['run']+manifest+['-p','ag1-protocol','--bin','protocol-codegen','--','--check']),
            ('typescript',['node',str(REFERENCE/'node_modules/typescript/bin/tsc'),'--strict','--target','ES2022','--module','commonjs','--moduleResolution','node','--rootDir','.','--outDir','.local/protocol-ts','--skipLibCheck','fixtures/protocol/codec-check.ts']),
            ('schema-roundtrip',['node','tools/check_protocol.mjs','.local/protocol-ts/fixtures/protocol/codec-check.js',str(evidence/'typescript-roundtrip.json')]),
            ('rust-roundtrip',cargo+['run']+manifest+['-p','ag1-protocol','--bin','protocol-codegen','--','--roundtrip',str(evidence/'typescript-roundtrip.json')]),
        ]
        for name,argv in checks:
            print(f'P03: executing {name}',flush=True);commands.append(run(argv,log=evidence/f'{name}.log',timeout=180))
        passed,measurements,log_bounds=validate_platform_log((evidence/'platform-tests.log').read_text())
        protocol_passed=validate_rust_log((evidence/'protocol-tests.log').read_text())
        remaining=owned_fixture_processes()
        if remaining:raise GateError(f'Fixture processes remain: {remaining}')
        commands.append(run(cargo+['metadata']+manifest+['--format-version','1'],log=evidence/'cargo-metadata.json'))
        metadata=read_json(evidence/'cargo-metadata.json')
        if next(p for p in metadata['packages'] if p['name']=='ag1-core')['dependencies']:raise GateError('Core acquired adapter dependencies')
        licenses=read_json(ROOT/'product/licenses/manifest.json')
        if licenses['cargo_lock_sha256']!=sha256(ROOT/'product/Cargo.lock'):raise GateError('Stale dependency license lock')
        actual={(p['name'],p['version']) for p in metadata['packages'] if p['source']}
        recorded={(p['name'],p['version']) for p in licenses['packages'] if 'scope' not in p}
        if actual!=recorded:raise GateError('Missing transitive dependency license')
        for package in licenses['packages']:verify_artifacts(ROOT,package['files'])
        baseline=read_json(ROOT/'implementation/evidence/P01/microbench-and-defects.json')['results']
        comparison={name:{key:baseline[name][key] for key in ['p50','p95']} for name in ['workspace_index_uncached_120_files','git_status','git_diff','read_tool_dispatch']}
        write_json(evidence/'measurements.json',{'platform':measurements,'log_bounds':log_bounds,'P01_ms':comparison,
            'interpretation':'Not matched trials: P03 hashes indexed files and supervises five Git calls with HEAD/worktree checks. P01 uses separate direct Git calls and different index/tool work. No parity speedup/release performance pass claimed. Synchronous durations include blocking; async ports use four bounded workers.'})
        capabilities={'linux':{'status':'native fixtures passed','kernel':platform.release(),'architecture':platform.machine()},
            'windows':{'status':'pending','required':['Job Object/tree shutdown','native pipe-close hang','PTY/ConPTY','contained paths and CAS']},
            'macos':{'status':'pending','required':['native descendant ownership/reaping','PTY lifecycle','contained paths and mutation']},
            'fake_state_machine':'Exit without pipe EOF triggers tree cancel after 200 ms; not a Windows native pass.'}
        write_json(evidence/'platform-capabilities.json',capabilities)
        verify_original();verify_dependencies('P03');verify_runtime_material(runtime,cache)
        if sha256(Path(lock['profile_file']))!=lock['profile_file_sha256'] or inputs()!=before:raise GateError('Protected runtime or gate inputs changed')
        snapshot=evidence/'delivered-snapshot';snapshot.mkdir(exist_ok=True)
        for relative in before:
            if relative.startswith(('product/','tools/','fixtures/protocol/')):
                destination=snapshot/relative;destination.parent.mkdir(parents=True,exist_ok=True);shutil.copyfile(ROOT/relative,destination)
        artifacts=[{'path':str(p.relative_to(ROOT)),'sha256':sha256(p)} for p in sorted(evidence.rglob('*')) if p.is_file() and p!=completion and '__pycache__' not in p.parts]
        verify_artifacts(ROOT,artifacts)
        write_json(completion,{'phase':'P03','status':'passed','completed_at':datetime.datetime.now(datetime.timezone.utc).isoformat(),
            'input_fingerprints':before,'dependencies':['P02'],'artifacts':artifacts,'commands':commands,
            'coverage':{'capability_foundations':['A37','A61','A62','A63','A64','A65','A66','A67','A68'],'invariants':['I18','I19','I21','I24','L06','L07'],
                'platform_tests':passed,'protocol_tests':protocol_passed,'codec_roundtrips':22,'remaining_owned_fixture_processes':remaining},
            'authority':{'before':'Original AG1 and externally owned llama services','after':'Unchanged. Rust services exercised disposable workspaces/owned fixture children only.'},
            'source_verification':original,'licenses':len(licenses['packages']),'platforms':capabilities,
            'pending_gates':['Windows/macOS native adapters and runners remain pending; unsupported modules are not exposed there.',
                'No real HTTP provider, OS credential store, agent execution, journal, Tauri desktop or model lifecycle introduced.',
                'CAS serializes cooperating writers and checks bytes before rename; unrelated external writers can race publication.',
                'Uninterruptible kernel tasks can exceed the 3-second cancel deadline: unresolved ownership, not successful cleanup.',
                'Release performance, arbitrary ACL/xattr preservation, production HTTP/secret adapters and native packaging remain later gates.'],
            'rollback':'Revert P03 additions and dependency/license graph together. Remove only disposable fixture paths. P01/P02 snapshots remain intact.',
            'next_phase_guarantees':'P04 can own a worker with explicit cwd/env, separate bounded stdout/stdin and diagnostic spool. P05/P07 have snapshots/revisions/CAS. P06 can reuse ownership without changing llama.cpp.'})
        print(f'P03 passed: {passed} native platform tests, {protocol_passed} protocol tests; zero remaining fixture children; runtimes preserved.')
    except Exception as exc:
        write_json(completion,{'phase':'P03','status':'failed','commands':commands,'error':str(exc),'observed_fixture_processes':owned_fixture_processes() if platform.system()=='Linux' else []})
        raise
