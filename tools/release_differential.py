"""P14 replay of immutable context/runtime oracles, with P14-only outputs."""
from __future__ import annotations

import json
import uuid

from common import ROOT, GateError, isolated_env, read_json, run, write_json
from context_gate import PROBES, validate_probe as context_probe
from runtime_differential import validate_inventory, validate_probe as runtime_probe
from tool_gate import environment


def differential():
    from release_gate import fingerprints, rust_results
    from reference_oracles import input_fingerprints

    evidence = ROOT / 'implementation/evidence/P14/differential' / uuid.uuid4().hex
    evidence.mkdir(parents=True)
    output = ROOT / 'implementation/evidence/P14/oracle-release-differential.json'
    before = fingerprints()
    reference = input_fingerprints()
    commands, observations = [], {}
    write_json(output, {'status': 'running', 'phase_complete': False})
    node, env = environment()
    env['CARGO_TARGET_DIR'] = str(ROOT / '.local/p14-target')
    source_env = isolated_env(ROOT / '.local/p14-replay' / evidence.name)
    source_env['PATH'] = str(node.parent) + ':' + source_env['PATH']
    source_env['NODE_OPTIONS'] = '--import=' + str(ROOT / 'tools/offline_guard.mjs')
    source_env['GIT_CEILING_DIRECTORIES'] = source_env['TMPDIR']
    try:
        for name in PROBES:
            log = evidence / f'context-{name}.log'
            commands.append(run([str(node), f'tools/probes/context_{name}_reference.mjs'],
                                env=source_env, log=log, timeout=1200))
            value = read_json(log)
            context_probe(name, value)
            observations['context-' + name] = value
        thinking_log = evidence / 'session-thinking-reference.log'
        commands.append(run([str(node), 'tools/release_thinking_reference.mjs'],
                            env=source_env, log=thinking_log, timeout=1200))
        thinking = json.loads(thinking_log.read_text().splitlines()[-1])
        if thinking != {'status': 'passed', 'provider_groups': 196, 'network_actions': 0, 'model_actions': 0}:
            raise GateError('Incomplete selected-thinking reference replay')
        observations['session-thinking'] = thinking
        inventory = read_json(ROOT / 'fixtures/runtime/differential-inventory.json')
        validate_inventory(inventory)
        for probe in inventory['probes']:
            name = probe['path'].rsplit('/', 1)[-1]
            log = evidence / (name + '.log')
            commands.append(run([str(node), probe['path']], env=source_env, log=log, timeout=1200))
            observations[name] = runtime_probe(probe, log.read_text())
        log = evidence / 'native-workspace.log'
        commands.append(run(['cargo', 'test', '--offline', '--locked', '--manifest-path',
                             'product/Cargo.toml', '--workspace'], env=env, log=log, timeout=3600))
        native = rust_results(log.read_text(), minimum=767)
        if fingerprints() != before or input_fingerprints() != reference:
            raise GateError('Release replay inputs changed during execution')
        report = {'status': 'passed', 'phase_complete': False, 'suite': 'release-differential',
                  'input_fingerprints': before, 'reference_fingerprints': reference,
                  'commands': commands, 'reference_probes': observations, 'native': native,
                  'model_actions': 0,
                  'scope': 'Fresh unchanged reference probes and complete native regressions; '
                           'offline fixtures include 2M-token context, recovery and B01–B04. '
                           'This is not packaged performance, live quality or duration acceptance.'}
        write_json(output, report)
        print(f'Release differential passed: {len(observations)} reference probes, {native["passed"]} native tests')
        return report
    except Exception as exc:
        write_json(output, {'status': 'failed', 'phase_complete': False,
                            'commands': commands, 'error': str(exc)})
        raise
