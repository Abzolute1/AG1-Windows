#!/usr/bin/env python3
"""Run real reference tests/probes and retain machine-verifiable evidence."""
from __future__ import annotations

import argparse
from pathlib import Path
import sys
import uuid

from common import ROOT, REFERENCE, EVIDENCE, GateError, isolated_env, read_json, run, sha256, verify_original, write_json


def validate_vitest(data: dict, *, expected: int | None = None) -> list[dict]:
    tests = [case for file in data.get('testResults', []) for case in file.get('assertionResults', [])]
    if not tests:
        raise GateError('Empty test selection cannot pass')
    if expected is not None and len(tests) != expected:
        raise GateError(f'Expected {expected} tests, received {len(tests)}')
    if data.get('success') is not True or data.get('numFailedTests', 0) or data.get('numPendingTests', 0):
        raise GateError('Reference report has failures, skipped/pending tests or unsuccessful execution')
    if data.get('numTotalTests') != len(tests) or data.get('numPassedTests') != len(tests):
        raise GateError('Summary count disagrees with actual executed assertions')
    if any(case.get('status') != 'passed' for case in tests):
        raise GateError('Every required test must execute and pass')
    return tests


def input_fingerprints():
    verify_original()
    source_manifest = read_json(ROOT / 'reference/source-files.json')
    for path, digest in source_manifest.items():
        if sha256(REFERENCE / path) != digest:
            raise GateError(f'Pinned executable reference changed: {path}')
    # Include executable output, not only source: stale dist must invalidate a run.
    import hashlib
    accumulator = hashlib.sha256()
    for file in sorted(REFERENCE.glob('packages/*/dist/**/*.js')):
        accumulator.update(str(file.relative_to(REFERENCE)).encode())
        accumulator.update(sha256(file).encode())
    scripts = {str(p.relative_to(ROOT)): sha256(p) for p in sorted((ROOT / 'tools').rglob('*'))
               if p.is_file() and p.suffix in ['.py', '.mjs']}
    return {'source_manifest': sha256(ROOT / 'reference/source-files.json'),
            'dist': accumulator.hexdigest(), 'scripts': scripts,
            'selected_paths': sha256(ROOT / 'research/evidence/ag1-selected-tests.txt'),
            'catalog_selection': sha256(ROOT / 'fixtures/catalog-selection.json'),
            'catalog_exceptions': sha256(ROOT / 'fixtures/catalog-exceptions.json'),
            'runtime_lock': sha256(ROOT / 'planning/evidence/llama-runtime-lock.json')}


def suite_selected(env, commands):
    selection = (ROOT / 'research/evidence/ag1-selected-tests.txt').read_text().splitlines()
    if not selection or len(selection) != len(set(selection)):
        raise GateError('Missing/duplicate selected test paths')
    groups = [('coding-agent', selection, 465), ('godot-agent-runtime', ['test'], 26)]
    total = 0
    for package, paths, expected in groups:
        cwd = REFERENCE / 'packages' / package
        for path in paths:
            if not (cwd / path).exists():
                raise GateError(f'Missing selected test fixture: {package}/{path}')
        report = EVIDENCE / f'{package}-tests.json'
        report.unlink(missing_ok=True)
        command = [str(REFERENCE / 'node_modules/.bin/vitest'), 'run', *paths,
                   '--maxWorkers=2', '--reporter=json', f'--outputFile={report}']
        commands.append(run(command, cwd=cwd, env=env, log=EVIDENCE / f'{package}-tests.log'))
        total += len(validate_vitest(read_json(report), expected=expected))
    return {'tests_passed': total, 'known_defects_not_corrected': True}


def validate_catalog_report(data, exceptions, *, reference=REFERENCE):
    tests = [(file, case) for file in data.get('testResults', []) for case in file.get('assertionResults', [])]
    if not tests or data.get('numTotalTests') != len(tests):
        raise GateError('Empty or inconsistent catalog test count')
    passed=0; accepted=[]
    for file, case in tests:
        path=Path(file['name']).resolve().relative_to(reference.resolve()).as_posix()
        exception=next((e for e in exceptions if e['source']==path and e['name']==case['fullName']),None)
        if exception:
            if sha256(reference/path)!=exception['source_sha256']:
                raise GateError(f'Stale exception source hash: {path}')
            if case['status']!=exception['expected_status']:
                raise GateError(f'Exception outcome changed; review {case["fullName"]}')
            if exception.get('required_error') and exception['required_error'] not in '\n'.join(case.get('failureMessages',[])):
                raise GateError('Known assertion failed for a different reason')
            accepted.append(exception)
        elif case['status']!='passed':
            raise GateError(f'Unexpected nonpassing test: {case["fullName"]}')
        else:
            passed+=1
    if not passed or data.get('numPassedTests')!=passed:
        raise GateError('Invalid catalog passing-test count')
    return {'passed':passed,'accepted_nonpassing':accepted}


def suite_catalog(env, commands):
    selection = read_json(ROOT / 'fixtures/catalog-selection.json')
    if not selection:
        raise GateError('Empty capability fixture catalog')
    failures = []; total = 0; accepted=[]
    exceptions=read_json(ROOT/'fixtures/catalog-exceptions.json')['exceptions']
    for package, paths in selection.items():
        if not paths:
            raise GateError(f'Empty catalog selection for {package}')
        cwd = REFERENCE / 'packages' / package
        for path in paths:
            if not (cwd / path).is_file():
                raise GateError(f'Missing catalog fixture: {package}/{path}')
        report = EVIDENCE / f'catalog-{package}.json'
        report.unlink(missing_ok=True)
        command = [str(REFERENCE / 'node_modules/.bin/vitest'), 'run', *paths,
                   '--maxWorkers=2', '--reporter=json', f'--outputFile={report}']
        try:
            # A failing original assertion remains a real exit 1. Only the exact
            # reviewed baseline exception below can establish a usable oracle.
            import subprocess,time
            log=EVIDENCE/f'catalog-{package}.log';start=time.monotonic()
            with log.open('w') as out:
                result=subprocess.run(command,cwd=cwd,env=env,stdout=out,stderr=subprocess.STDOUT,timeout=900)
            commands.append({'argv':command,'cwd':str(cwd),'exit_code':result.returncode,'seconds':time.monotonic()-start,'log':str(log.relative_to(ROOT))})
            if result.returncode not in [0,1]:raise GateError('Catalog runner crashed')
            checked=validate_catalog_report(read_json(report),exceptions)
            if result.returncode and not any(e['expected_status']=='failed' for e in checked['accepted_nonpassing']):
                raise GateError('Nonzero runner status without the named baseline assertion')
            total+=checked['passed'];accepted.extend(checked['accepted_nonpassing'])
        except GateError as exc:
            failures.append(str(exc))
    if failures:
        raise GateError('Capability catalog did not pass: ' + '; '.join(failures))
    if {e['name'] for e in accepted}!={e['name'] for e in exceptions}:
        raise GateError('Expected catalog exception not exercised')
    return {'tests_passed': total, 'packages': len(selection),'expected_stale_assertions':sum(e['expected_status']=='failed' for e in accepted),'pending_live_or_platform_tests':sum(e['expected_status']=='skipped' for e in accepted),'all_original_tests_passed':False}


def suite_session(env, commands):
    output=ROOT/'fixtures/traces/session.json'
    commands.append(run(['node',str(ROOT/'tools/probes/capture_session.mjs'),str(REFERENCE),str(output)],env=env,log=EVIDENCE/'session-capture.log'))
    data=read_json(output)
    if len(data.get('scenarios',[]))!=2 or data.get('modelActions')!=0:
        raise GateError('Missing session projection success/failure captures')
    return {'scenarios':2,'requests':8,'real_model_actions':0,'replacement_for':'R01 stale cross-turn projection assertion'}


def validate_defects(probes: dict, fork: dict):
    cap = probes.get('aggregateToolCap', {})
    memory = probes.get('corruptMemory', {})
    race = probes.get('sourceReadRevisionRace', {})
    observed = {
        'B01': cap.get('limitChars') == 400 and cap.get('actualTextChars') == 600,
        'B02': memory.get('loadReturnedEntries') == 0 and memory.get('quarantineCreated') is False and memory.get('afterUpdate', {}).get('entries') == [],
        'B03': all(race.get(k) is True for k in ['injected', 'returnedOldEvidence', 'currentFileContainsNew']) and race.get('claimedRevisionStatus') == 'current',
        'B04': fork.get('referenceSurvives') is True and fork.get('parentReadable') == 'Required historical evidence' and fork.get('childReadable') is False,
    }
    if not all(observed.values()):
        raise GateError(f'Known-defect baseline changed or failed to reproduce: {observed}')
    return {'observed_known_defects': observed, 'target_status': 'All four corrected behavior requirements remain failing in immutable TS reference; not fixes.'}


def suite_defects(env, commands):
    micro = EVIDENCE / 'microbench-and-defects.json'
    fork = EVIDENCE / 'fork-defect.json'
    for script, output in [('measure-reference.mjs', micro), ('fork-reference.mjs', fork)]:
        output.unlink(missing_ok=True)
        commands.append(run(['node', str(ROOT / 'tools/probes' / script), str(REFERENCE), str(output)],
                            env=env, log=EVIDENCE / f'{script}.log'))
    return validate_defects(read_json(micro)['probes'], read_json(fork))


def suite_llama(env, commands):
    commands.append(run(['node', str(ROOT / 'tools/probes/capture_llama.mjs'), str(REFERENCE), str(ROOT / 'fixtures/llama')],
                        env=env, log=EVIDENCE / 'llama-capture.log'))
    plans = read_json(ROOT / 'fixtures/llama/launch-plans.json')
    payloads = read_json(ROOT / 'fixtures/llama/request-payloads.json')
    if len(plans.get('captures', [])) != 9 or len(payloads.get('payloads', [])) != 6:
        raise GateError('Incomplete model launch/request fixture capture')
    if any(plans['modelActions'].values()):
        raise GateError('Unexpected real model action')
    return {'launch_plans': 9, 'request_payloads': 6, 'real_model_actions': 0}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--suite', required=True, choices=['selected', 'known-defects', 'llama', 'catalog', 'session', 'context-differential'])
    parser.add_argument('--offline', action='store_true', required=True)
    args = parser.parse_args()
    if args.suite == 'context-differential':
        from context_gate import differential
        differential()
        return
    fingerprints = input_fingerprints()
    run_id = uuid.uuid4().hex
    environment = isolated_env(ROOT / '.local/runs' / run_id)
    environment['NODE_OPTIONS'] = f'--import={ROOT / "tools/offline_guard.mjs"}'
    commands = []
    output = EVIDENCE / f'oracle-{args.suite}.json'
    # Replace old passed status before starting, so interruption cannot leave a fresh pass.
    write_json(output, {'status': 'running', 'run_id': run_id, 'input_fingerprints': fingerprints})
    try:
        fn = {'selected': suite_selected, 'known-defects': suite_defects, 'llama': suite_llama, 'catalog': suite_catalog, 'session':suite_session}[args.suite]
        result = fn(environment, commands)
        if input_fingerprints() != fingerprints:
            raise GateError('Inputs changed while the oracle was running')
        write_json(output, {'status': 'passed', 'run_id': run_id, 'suite': args.suite,
                            'input_fingerprints': fingerprints, 'commands': commands, 'result': result})
        print(f'{args.suite}: {result}')
    except Exception as exc:
        write_json(output, {'status': 'failed', 'run_id': run_id, 'suite': args.suite,
                            'input_fingerprints': fingerprints, 'commands': commands, 'error': str(exc)})
        raise


if __name__ == '__main__':
    try:
        main()
    except GateError as exc:
        print(f'FAILED: {exc}', file=sys.stderr)
        raise SystemExit(1)
