"""P09 fresh tool/reference execution. Audit success alone is not phase completion."""
from __future__ import annotations
import argparse
import datetime
import json
import os
from pathlib import Path
import re
import shutil

from common import ROOT, PIN, GateError, read_json, write_json, sha256, run, verify_original
from provider_gate import validate_rust_log
from tool_coverage import selected_cases, validate_coverage


def inputs():
    files = []
    for base in ['product/crates/ag1-core', 'product/crates/ag1-platform',
                 'product/crates/ag1-storage', 'product/compat/extensions',
                 'product/licenses', 'fixtures/tools']:
        files.extend(p for p in (ROOT / base).rglob('*') if p.is_file())
    files.extend((ROOT / 'tools/probes').glob('tool_*'))
    files.extend((ROOT / 'tools/tests').glob('test_tool*.py'))
    files.extend(ROOT / p for p in ['product/Cargo.toml', 'product/Cargo.lock',
        'tools/tool_gate.py', 'tools/tool_coverage.py', 'tools/common.py',
        'tools/tool_measurement.py', 'tools/tool_licenses.py', 'tools/provider_gate.py',
        'tools/phase_gate.py', 'tools/prepare_extension_host.py',
        'tools/record_protocol_licenses.py', 'tools/offline_guard.mjs',
        'phases/P09-tools-policy-and-resources.md'])
    return {p.relative_to(ROOT).as_posix(): sha256(p) for p in sorted(set(files))}


def environment():
    node = ROOT / '.local/toolchains/node-v24.14.0-linux-x64/bin/node'
    # Preserve the user's home path; do not forward ambient provider credentials.
    env = {k: os.environ[k] for k in ['HOME', 'PATH', 'CARGO_HOME', 'RUSTUP_HOME'] if k in os.environ}
    env.update(PATH=str(node.parent) + ':' + env.get('PATH', '/usr/bin:/bin'),
               NO_COLOR='1', CI='1', LANG='C.UTF-8', TZ='UTC', AG1_LLAMA_CPP='0')
    return node, env


def rust_cases(path, log, count):
    actual = validate_rust_log(log, count)
    if actual != count:
        raise GateError(f'Rust inventory changed: {path}: {actual} != {count}')
    return {(path, name) for name in re.findall(r'^test ([a-zA-Z_][a-zA-Z0-9_:]*) \.\.\. ok$', log, re.M)}


def node_cases(path, log):
    counts = {key: re.findall(r'^# ' + key + r' (\d+)$', log, re.M)
              for key in ['tests', 'pass', 'fail', 'cancelled', 'skipped', 'todo']}
    if any(len(value) != 1 for value in counts.values()):
        raise GateError('Missing or ambiguous helper test summary')
    counts = {key: int(value[0]) for key, value in counts.items()}
    names = re.findall(r'^ok \d+ - (.+)$', log, re.M)
    if (not counts['tests'] or counts['tests'] != counts['pass'] or
        any(counts[k] for k in ['fail', 'cancelled', 'skipped', 'todo']) or
        len(names) != counts['pass'] or len(names) != len(set(names))):
        raise GateError('Skipped, failed or incomplete helper tests')
    return {(path, name) for name in names}


def probe_result(log):
    rows = []
    for line in log.splitlines():
        try:
            value = json.loads(line)
        except json.JSONDecodeError:
            continue
        if isinstance(value, dict) and value.get('status') == 'passed':
            rows.append(value)
    if len(rows) != 1:
        raise GateError('Missing or ambiguous reference probe completion')
    value = rows[0]
    if any(value[k] != 0 for k in value if k.endswith('_actions') or k in ['external_commands', 'package_scripts']):
        raise GateError('Unexpected external probe action')
    if not any(type(v) is int and v > 0 for v in value.values()):
        raise GateError('Empty reference probe')
    return value


def audit():
    from phase_gate import verify_dependencies
    evidence = ROOT / 'implementation/evidence/P09'
    evidence.mkdir(parents=True, exist_ok=True)
    report = evidence / 'audit.json'
    commands, passed, probes, contracts = [], set(), {}, {}
    write_json(report, {'phase': 'P09', 'status': 'running', 'phase_complete': False})
    node, env = environment()
    def execute(name, argv):
        print('P09 audit: ' + name, flush=True)
        log = evidence / (name + '.log')
        commands.append(run(argv, env=env, log=log, timeout=1200))
        return log.read_text()
    try:
        verify_dependencies('P09')
        verify_original()
        execute('helper-dependencies', ['python3', 'tools/prepare_extension_host.py'])
        before = inputs()
        inventory = read_json(ROOT / 'fixtures/tools/gate-inventory.json')
        if inventory.get('version') != 1 or len(inventory['probes']) != 52 or len(inventory['contracts']) != 19:
            raise GateError('Tool gate inventory changed')
        execute('format', ['cargo', 'fmt', '--manifest-path', 'product/Cargo.toml', '--all', '--', '--check'])
        execute('clippy', ['cargo', 'clippy', '--offline', '--manifest-path', 'product/Cargo.toml',
                '-p', 'ag1-core', '-p', 'ag1-platform', '-p', 'ag1-storage', '--all-targets', '--', '-D', 'warnings'])
        execute('coverage-validator', ['python3', '-m', 'unittest', 'discover', '-s', 'tools/tests', '-p', 'test_tool_coverage.py', '-v'])
        execute('gate-validator', ['python3', '-m', 'unittest', 'discover', '-s', 'tools/tests', '-p', 'test_tool_gate.py', '-v'])
        for probe in inventory['probes']:
            path = probe['path']
            name = Path(path).stem + ''.join('-' + arg.removeprefix('--') for arg in probe['arguments'])
            probes[name] = probe_result(execute(name, [str(node), path, *probe['arguments']]))
        for contract in inventory['contracts']:
            package, suite = contract['package'], contract['suite']
            path = f'product/crates/{package}/tests/{suite}.rs'
            log = execute(package + '-' + suite, ['cargo', 'test', '--offline', '--manifest-path',
                          'product/Cargo.toml', '-p', package, '--test', suite])
            fresh = rust_cases(path, log, contract['count'])
            passed.update(fresh)
            contracts[path] = len(fresh)
        for path in sorted((ROOT / 'product/compat/extensions/test').glob('*.test.mjs')):
            relative = path.relative_to(ROOT).as_posix()
            log = execute('helper-' + path.stem, [str(node), '--test', '--test-reporter=tap', relative])
            fresh = node_cases(relative, log)
            passed.update(fresh)
            contracts[relative] = len(fresh)
        log = execute('driver', ['python3', '-m', 'unittest', 'discover', '-s', 'tools/tests', '-p', 'test_tool_driver.py', '-v'])
        names = re.findall(r'^(test_\w+) \([^\n]+\) \.\.\. ok$', log, re.M)
        if len(names) != 4 or not re.search(r'\nRan 4 tests in [^\n]+\n\nOK\n', log):
            raise GateError('Missing native deterministic driver tests')
        passed.update(('tools/tests/test_tool_driver.py', name) for name in names)
        coverage = validate_coverage(read_json(ROOT / 'fixtures/tools/case-coverage.json'), selected_cases(), passed)
        verify_original()
        verify_dependencies('P09')
        if before != inputs():
            raise GateError('Tool inputs changed during audit')
        result = {'phase': 'P09', 'status': 'passed', 'phase_complete': False,
                  'completed_at': datetime.datetime.now(datetime.timezone.utc).isoformat(),
                  'source_commit': PIN, 'input_fingerprints': before, 'commands': commands,
                  'coverage': coverage, 'contracts': contracts, 'reference_probes': probes,
                  'passed_cases': [{'path': path, 'test': test} for path, test in sorted(passed)]}
        write_json(report, result)
        print(json.dumps({'status': 'passed', 'phase_complete': False, 'coverage': coverage}))
        return result
    except Exception as error:
        write_json(report, {'phase': 'P09', 'status': 'failed', 'phase_complete': False,
                            'commands': commands, 'error': str(error)})
        raise


def execute_p09():
    from phase_gate import verify_dependencies, verify_artifacts, verify_runtime_material
    from tool_measurement import measure
    from tool_licenses import verify as verify_licenses
    evidence = ROOT / 'implementation/evidence/P09'
    evidence.mkdir(parents=True, exist_ok=True)
    completion = evidence / 'completion.json'
    commands = []
    write_json(completion, {'phase': 'P09', 'status': 'running'})
    _, env = environment()
    def execute(name, argv):
        print('P09: ' + name, flush=True)
        commands.append(run(argv, env=env, log=evidence / (name + '.log'), timeout=1200))
    try:
        verify_dependencies('P09')
        original = verify_original()
        execute('native-license-capture', ['python3', 'tools/record_protocol_licenses.py'])
        licenses = verify_licenses()
        write_json(evidence / 'licenses.json', licenses)
        before = inputs()
        report = audit()
        commands.extend(report['commands'])
        if report['input_fingerprints'] != before:
            raise GateError('Tool audit differs from phase inputs')
        execute('measurement-validator', ['python3', '-m', 'unittest', 'discover', '-s', 'tools/tests', '-p', 'test_tool_measurement.py', '-v'])
        execute('measurement-build', ['cargo', 'build', '--offline', '--manifest-path', 'product/Cargo.toml', '-p', 'ag1-platform', '--release', '--example', 'tool-probe', '--bin', 'ag1-process-host'])
        performance = measure(ROOT / 'product/target/release/examples/tool-probe', evidence, env)
        for row in performance['runs']:
            commands.append({key: row[key] for key in ['argv', 'exit_code', 'seconds', 'log']})
        verify_licenses()
        verify_original()
        verify_dependencies('P09')
        verify_runtime_material(read_json(ROOT / 'implementation/evidence/P01/runtime-manifest.json'), read_json(ROOT / '.local/runtime/hash-cache.json'))
        runtime_lock = read_json(ROOT / 'planning/evidence/llama-runtime-lock.json')
        if sha256(Path(runtime_lock['profile_file'])) != runtime_lock['profile_file_sha256']:
            raise GateError('Tuned llama profile changed')
        if inputs() != before:
            raise GateError('Tool implementation changed during the phase gate')
        limitations = [
            'TS remains desktop/session authority. P09 delivers native tools, approval/policy/resource APIs, target capability interfaces and the optional extension host; P10 composes the session loop and command callbacks, P11 implements concrete check/browser/native targets, and P12 supplies client routing and rendering.',
            'Every selected Linux case is accounted for: 467 covered and 24 intentional corrections. Two originally skipped Windows shell-close cases remain explicitly assigned to P14; they are not reported as passing.',
            'P05 requires original result custody even for explicit artifact pages. Pages retain their content and do not recursively create text artifacts. Native successful check completion and verification-lease release commit atomically; hooks cannot forge success and old receipts cannot release newer leases.',
            'Sandbox execution requires the admitted native backend. Missing/failing containment never silently falls back to unrestricted execution. Native direct argv, held directory descriptors, filtered PATH and no-login shell behavior have documented compatibility corrections.',
            'Extension code is trusted host-account code, not an OS sandbox. Entry bytes are hash-bound; relative imports remain trusted dependencies. The compatibility table explicitly records terminal/component and callback-bearing APIs that are unsupported, and later session/client composition. There is no hidden AgentSession in the helper.',
            'Custom provider hooks use native scheduling, private credential grants, actual HTTP or granted custom streams, bounded reentrant callbacks and acknowledged catalog refresh. Tests use owned HTTP/TLS fixtures and fixture credentials; no live model or real account is used.',
            'Web display comparisons cover exact text content and compact/expanded semantics; they do not certify P12 desktop framing or visual interaction. No desktop was launched by this gate.',
            'Synthetic measurements separate pure preparation, durable dispatch, native file I/O, large shell custody and optional helper roundtrip. Memory is a sampled process-tree observation, with separately tested admission/queue/frame/output limits. No TS speedup, whole-application responsiveness, model throughput or other-platform performance is claimed.',
            'The unchanged Jiti distribution matches its published archive. The conservative bundled dependency closure includes build-aliased dependencies. One archive supplies only an MIT declaration and author metadata; original metadata and labeled supplemental permission text are retained without inventing a copyright notice.',
            'All three tuned llama.cpp executables, libraries, GGUFs and original profile settings were reverified unchanged. Live per-model quality, other operating systems, migration/cutover, packaging and 8h/24h release workloads remain their explicit later gates.',
        ]
        coverage = {'capabilities': ['A16','A59','A60','A61','A62','A63','A64','A65','A66','A68','A71'],
                    'selected_cases': report['coverage'], 'contracts': report['contracts'],
                    'reference_probes': len(report['reference_probes']), 'native_driver_tests': 4,
                    'regressions': ['B01 aggregate result custody', 'D03 extension execution and parent authority'],
                    'foundations': ['A12','A26','A67','A69','A70','A72']}
        write_json(evidence / 'coverage.json', coverage)
        write_json(evidence / 'limitations.json', limitations)
        (evidence / 'REPORT.md').write_text(
            '# P09 tools, policy, resources and extension execution\n\n'
            'The executed gate covers 52 immutable SDK probe runs, 19 Rust contract suites, three helper test files, four real native deterministic driver tests, validator rejection tests, formatting, strict Clippy, dependency notices and three synthetic measurement runs. Exact case evidence and intentional changes are in [coverage](coverage.json), [case audit](audit.json), [measurements](performance.json), and [limitations](limitations.json).\n\n'
            'P09 keeps one acknowledged intent before a tool effect and persists the native result before hook projection and terminal acknowledgement. Approval tickets bind scope, arguments, epoch, policy and target revision. Replayed completion never repeats an effect; uncertain intent remains explicit. Native adapters exercise files, search, Git, shell/jobs, sandbox, web, resources/packages, images, target interfaces and trusted extensions in disposable workspaces.\n\n'
            'API and compatibility guides are retained in the delivered snapshot: ag1-core/TOOLS.md, ag1-platform/TOOL_COMPATIBILITY.md, RESOURCES.md, WEB.md and compat/extensions/README.md. P10 receives executable services and their explicit ownership/composition contracts.\n\n'
            'Rollback: stop only owned fixture jobs/helpers, preserve uncertain operation records and original evidence, and retain the preceding P08 checkpoint. The executing TypeScript desktop and original llama.cpp installations retain authority throughout P09.\n\n' + '\n'.join('- ' + item for item in limitations) + '\n')
        for relative in before:
            target = evidence / 'delivered-snapshot' / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / relative, target)
        artifacts = [{'path': p.relative_to(ROOT).as_posix(), 'sha256': sha256(p)}
                     for p in sorted(evidence.rglob('*')) if p.is_file() and p != completion]
        verify_artifacts(ROOT, artifacts)
        write_json(completion, {'phase': 'P09', 'status': 'passed',
            'completed_at': datetime.datetime.now(datetime.timezone.utc).isoformat(),
            'input_fingerprints': {'product': before, 'original': original},
            'dependencies': read_json(ROOT / 'planning/phase-graph.json')['P09']['dependencies'],
            'artifacts': artifacts, 'commands': commands, 'coverage': coverage,
            'authority': {'before': 'TypeScript desktop/session and existing externally owned llama.cpp runtimes',
                          'after': 'Unchanged; native P09 services executed only against isolated fixtures'},
            'pending_gates': limitations})
        print('P09 passed: selected tool compatibility, durable native execution, notices and measurements verified.', flush=True)
    except Exception as error:
        write_json(completion, {'phase': 'P09', 'status': 'failed', 'commands': commands, 'error': str(error),
                               'authority': 'Original sources, desktop authority and tuned runtimes unchanged'})
        raise


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--audit', action='store_true', required=True)
    parser.parse_args()
    audit()
