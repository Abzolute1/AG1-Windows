"""Executed P10 source/native contracts. A passing replay is not phase completion."""
from __future__ import annotations

import json
from pathlib import Path
import uuid

from common import ROOT, PIN, GateError, read_json, write_json, sha256, run, isolated_env, verify_original
from tool_gate import environment, probe_result, rust_cases


def inputs():
    files = []
    for base in ['product/crates/ag1-core', 'product/crates/ag1-platform',
                 'product/crates/ag1-storage', 'product/crates/ag1-protocol',
                 'product/compat/extensions', 'product/licenses', 'fixtures/runtime']:
        files.extend(p for p in (ROOT / base).rglob('*') if p.is_file())
    files.extend((ROOT / 'tools/probes').glob('runtime_*.mjs'))
    files.extend((ROOT / 'tools').glob('runtime_*.py'))
    files.extend((ROOT / 'tools/tests').glob('test_runtime*.py'))
    files.extend(ROOT / p for p in ['tools/phase_gate.py','tools/record_protocol_licenses.py','product/LICENSE','implementation/README.md','fixtures/runtime/scope.json'])
    files.extend(ROOT / p for p in ['product/Cargo.toml', 'product/Cargo.lock',
        'tools/runtime_differential.py', 'tools/reference_oracles.py', 'tools/common.py',
        'tools/tool_gate.py', 'tools/provider_gate.py', 'tools/offline_guard.mjs',
        'tools/tests/test_runtime_differential.py', 'phases/P10-session-runtime-and-recovery.md'])
    return {p.relative_to(ROOT).as_posix(): sha256(p) for p in sorted(set(files))}


def validate_inventory(inventory):
    if inventory.get('version') != 1 or inventory.get('phase') != 'P10' or inventory.get('phase_complete') is not False:
        raise GateError('Invalid runtime differential scope')
    probes, contracts = inventory.get('probes', []), inventory.get('contracts', [])
    expected = {p.relative_to(ROOT).as_posix() for p in (ROOT / 'tools/probes').glob('runtime_*_reference.mjs')}
    if not probes or len(probes) != len(expected) or {p.get('path') for p in probes} != expected:
        raise GateError('Missing or duplicate runtime probes')
    suites = {('ag1-core', 'runtime_contract'), ('ag1-platform', 'session_contract'),
              ('ag1-platform', 'registry_contract'), ('ag1-storage', 'runtime_storage_contract')}
    if len(contracts) != len(suites) or {(c.get('package'), c.get('suite')) for c in contracts} != suites:
        raise GateError('Incomplete runtime contract inventory')
    for contract in contracts:
        if type(contract.get('count')) is not int or contract['count'] <= 0:
            raise GateError('Empty runtime contract')
    for probe in probes:
        path = Path(probe.get('oracle', ''))
        if path.is_absolute() or path.parent != Path('fixtures/runtime') or path.suffix != '.json':
            raise GateError('Invalid runtime oracle path')
        fixture = read_json(ROOT / path)
        if fixture.get('source_commit') != PIN or fixture.get('model_actions') != 0 or fixture.get('network_actions') != 0:
            raise GateError('Unpinned or live runtime oracle')
        validate_probe(probe, json.dumps(probe.get('expected')))


def validate_probe(probe, log):
    actual = probe_result(log)
    if actual != probe.get('expected') or actual.get('model_actions') != 0 or actual.get('network_actions') != 0:
        raise GateError(f'Runtime probe result changed: {probe.get("path")}: {actual}')
    return actual


def differential():
    from reference_oracles import input_fingerprints
    evidence = ROOT / 'implementation/evidence/P10'
    output = evidence / 'oracle-runtime-differential.json'
    commands, probes, contracts = [], {}, {}
    run_id = uuid.uuid4().hex
    write_json(output, {'status': 'running', 'phase_complete': False, 'run_id': run_id})
    try:
        verify_original()
        inventory = read_json(ROOT / 'fixtures/runtime/differential-inventory.json')
        validate_inventory(inventory)
        before = {'product': inputs(), 'reference': input_fingerprints()}
        node, rust_env = environment()
        source_env = isolated_env(ROOT / '.local/runs' / run_id)
        source_env['PATH'] = str(node.parent) + ':' + source_env['PATH']
        source_env['NODE_OPTIONS'] = '--import=' + str(ROOT / 'tools/offline_guard.mjs')
        # Disposable nonrepo fixtures must not discover AG1's enclosing checkout
        # when TMPDIR is below .local. Repositories inside fixtures still work.
        source_env['GIT_CEILING_DIRECTORIES'] = source_env['TMPDIR']
        for probe in inventory['probes']:
            name = Path(probe['path']).stem
            print('P10 differential: ' + name, flush=True)
            log = evidence / (name + '.log')
            commands.append(run([str(node), probe['path']], env=source_env, log=log, timeout=1200))
            probes[name] = validate_probe(probe, log.read_text())
        for contract in inventory['contracts']:
            package, suite = contract['package'], contract['suite']
            path = f'product/crates/{package}/tests/{suite}.rs'
            print('P10 differential: ' + package + '/' + suite, flush=True)
            log = evidence / (package + '-' + suite + '.log')
            commands.append(run(['cargo', 'test', '--offline', '--manifest-path', 'product/Cargo.toml',
                                 '-p', package, '--test', suite], env=rust_env, log=log, timeout=1200))
            cases = rust_cases(path, log.read_text(), contract['count'])
            contracts[path] = sorted(name for _, name in cases)
        after = {'product': inputs(), 'reference': input_fingerprints()}
        if before != after:
            raise GateError('Runtime inputs changed during differential replay')
        report = {'status': 'passed', 'phase_complete': False, 'run_id': run_id,
                  'suite': 'runtime-differential', 'input_fingerprints': after,
                  'commands': commands, 'reference_probes': probes, 'contracts': contracts,
                  'model_actions': 0, 'network_scope': 'loopback fake providers and disposable process fixtures only'}
        write_json(output, report)
        print(json.dumps({'status': 'passed', 'phase_complete': False, 'probes': len(probes),
                          'contracts': sum(len(c) for c in contracts.values()), 'model_actions': 0}))
        return report
    except Exception as exc:
        write_json(output, {'status': 'failed', 'phase_complete': False, 'run_id': run_id,
                            'commands': commands, 'error': str(exc)})
        raise
