"""Executable P02 validation; all production authority stays with the original AG1."""
import datetime
import json
import re
import shutil
from pathlib import Path

from common import ROOT, REFERENCE, GateError, read_json, write_json, sha256, run, verify_original


def validate_rust_log(log: str, minimum=18):
    summaries = re.findall(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured; (\d+) filtered out;', log)
    if len(summaries) != 1:
        raise GateError('Missing or ambiguous protocol test summary')
    passed, failed, ignored, measured, filtered = map(int, summaries[0])
    if passed < minimum or any([failed, ignored, measured, filtered]):
        raise GateError('Empty, incomplete or skipped protocol tests')
    cases = re.findall(r'^test ([a-z_]+) \.\.\. ok$', log, flags=re.M)
    if len(cases) != passed or len(set(cases)) != passed:
        raise GateError('Test names and summary differ')
    return passed


def inputs():
    files = [p for p in (ROOT/'product').rglob('*') if p.is_file() and 'target' not in p.relative_to(ROOT/'product').parts]
    files += [p for p in (ROOT/'fixtures/protocol').rglob('*') if p.is_file()]
    files += [ROOT/'tools'/p for p in ['protocol_gate.py', 'phase_gate.py', 'common.py', 'check_protocol.mjs', 'record_protocol_licenses.py', 'tests/test_protocol_gate.py']]
    files += [ROOT/'implementation/evidence/P01/completion.json', ROOT/'phases/P02-protocol-and-core-boundary.md']
    return {str(p.relative_to(ROOT)): sha256(p) for p in sorted(files)}


def execute_p02():
    # Import only at execution time so the shared registry can dispatch here.
    from phase_gate import verify_artifacts, verify_dependencies, verify_runtime_material
    evidence = ROOT/'implementation/evidence/P02'; evidence.mkdir(parents=True, exist_ok=True)
    completion = evidence/'completion.json'; commands = []
    write_json(completion, {'phase': 'P02', 'status': 'running'})
    try:
        verify_dependencies('P02'); original = verify_original()
        runtime = read_json(ROOT/'implementation/evidence/P01/runtime-manifest.json')
        verify_runtime_material(runtime, read_json(ROOT/'.local/runtime/hash-cache.json'))
        lock = read_json(ROOT/'planning/evidence/llama-runtime-lock.json')
        if sha256(Path(lock['profile_file'])) != lock['profile_file_sha256']:
            raise GateError('Original model profile changed')
        before = inputs()
        cargo = ['cargo', '--offline']
        manifest = ['--manifest-path', 'product/Cargo.toml', '--locked']
        checks = [
            ('gate-tests', ['python3', '-m', 'unittest', 'discover', '-s', 'tools/tests', '-v']),
            ('format', ['cargo', 'fmt', '--manifest-path', 'product/Cargo.toml', '--all', '--check']),
            ('clippy', cargo + ['clippy'] + manifest + ['--workspace', '--all-targets', '--', '-D', 'warnings']),
            ('protocol-tests', cargo + ['test'] + manifest + ['-p', 'ag1-protocol', '--test', 'protocol_contract', '--', '--nocapture']),
            ('generated-drift', cargo + ['run'] + manifest + ['-p', 'ag1-protocol', '--bin', 'protocol-codegen', '--', '--check']),
            ('typescript', ['node', str(REFERENCE/'node_modules/typescript/bin/tsc'), '--strict', '--target', 'ES2022', '--module', 'commonjs', '--moduleResolution', 'node', '--rootDir', '.', '--outDir', '.local/protocol-ts', '--skipLibCheck', 'fixtures/protocol/codec-check.ts']),
            ('schema-roundtrip', ['node', 'tools/check_protocol.mjs', '.local/protocol-ts/fixtures/protocol/codec-check.js', str(evidence/'typescript-roundtrip.json')]),
            ('rust-roundtrip', cargo + ['run'] + manifest + ['-p', 'ag1-protocol', '--bin', 'protocol-codegen', '--', '--roundtrip', str(evidence/'typescript-roundtrip.json')]),
        ]
        for name, argv in checks:
            print(f'P02: executing {name}', flush=True)
            commands.append(run(argv, log=evidence/f'{name}.log'))
        log = (evidence/'protocol-tests.log').read_text()
        passed = validate_rust_log(log)
        measurement = re.search(r'^P02_MEASUREMENT (.+)$', log, flags=re.M)
        peak = re.search(r'30s simulated pause: peak transient bytes=(\d+)', log)
        if not measurement or not peak or int(peak[1]) > 1024*1024:
            raise GateError('Missing or overflowing queue measurements')
        measurements = json.loads(measurement[1]); measurements['peak_subscriber_bytes'] = int(peak[1])
        if measurements['dispatches'] != 1000 or measurements['events'] != 2000:
            raise GateError('Incomplete fixture dispatch measurement')
        codecs = json.loads((evidence/'schema-roundtrip.log').read_text())
        if codecs['roundtrips'] < 22 or codecs['schema_rejections'] < 10:
            raise GateError('Incomplete codec selection')
        licenses = read_json(ROOT/'product/licenses/manifest.json')
        if licenses['cargo_lock_sha256'] != sha256(ROOT/'product/Cargo.lock') or len(licenses['packages']) != 39:
            raise GateError('Dependency license record does not match locked graph')
        for package in licenses['packages']:
            verify_artifacts(ROOT, package['files'])
        # Core stays adapter- and GUI-independent. Lock metadata is checked as well as source files.
        metadata_path = evidence/'cargo-metadata.json'
        commands.append(run(cargo + ['metadata'] + manifest + ['--format-version', '1'], log=metadata_path))
        metadata = read_json(metadata_path)
        core = next(p for p in metadata['packages'] if p['name'] == 'ag1-core')
        if core['dependencies']:
            raise GateError('Core foundation acquired unexpected adapter dependencies')
        actual = {(p['name'],p['version']) for p in metadata['packages'] if p['source']}
        licensed = {(p['name'],p['version']) for p in licenses['packages'] if 'scope' not in p}
        if actual != licensed:
            raise GateError('Unrecorded transitive dependency')
        verify_original(); verify_dependencies('P02')
        verify_runtime_material(runtime, read_json(ROOT/'.local/runtime/hash-cache.json'))
        if sha256(Path(lock['profile_file'])) != lock['profile_file_sha256'] or inputs() != before:
            raise GateError('Inputs changed during protocol gate')
        write_json(evidence/'measurements.json', measurements)
        # Future phases can edit product/tools without invalidating this immutable completion checkpoint.
        snapshot = evidence/'delivered-snapshot'; snapshot.mkdir(exist_ok=True)
        delivered = []
        for relative in before:
            if relative.startswith(('product/', 'fixtures/protocol/', 'tools/')):
                target = snapshot/relative; target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(ROOT/relative, target); delivered.append(str(target.relative_to(ROOT)))
        files = [p for p in evidence.rglob('*') if p.is_file() and p != completion and '__pycache__' not in p.parts]
        artifacts = [{'path': str(p.relative_to(ROOT)), 'sha256': sha256(p)} for p in sorted(files)]
        verify_artifacts(ROOT, artifacts)
        write_json(completion, {'phase': 'P02', 'status': 'passed', 'completed_at': datetime.datetime.now(datetime.timezone.utc).isoformat(),
            'input_fingerprints': before, 'dependencies': ['P01'], 'artifacts': artifacts, 'commands': commands,
            'coverage': {'capability_foundations': ['A02','A26','A41','A42','A59','A60','A73','A75','A78'], 'invariants': ['I01','I11','I20','I22','L05','L06','L07'], 'rust_contract_tests': passed, 'codec': codecs},
            'authority': {'before': 'Original AG1 and external llama services', 'after': 'Unchanged; Rust host is an explicitly fake in-memory fixture'},
            'source_verification': original, 'delivered_checkpoint_files': len(delivered), 'measurements': measurements,
            'pending_gates': ['P03 platform adapters, P04 Tauri/React desktop and TS worker remain unimplemented.', 'P05 durable command/event storage and restart reconciliation remain unimplemented; this fake host discards state on restart.', 'No filesystem durability, native IPC authorization, real inference, model quality or production performance claim.', 'P01 pending live/provider/platform gates remain pending.'],
            'rollback': 'Remove/revert only product foundation and fixtures/protocol together; originals and runtime authority never changed.',
            'next_phase_guarantees': 'P03 receives GUI-independent ports; P04 receives checked Rust DTOs/schema/TS and a deterministic fake host.'})
        print(f'P02 passed: {passed} Rust contract tests, {codecs["roundtrips"]} cross-language round trips; no runtime actions.')
    except Exception as exc:
        write_json(completion, {'phase': 'P02', 'status': 'failed', 'commands': commands, 'error': str(exc)})
        raise
