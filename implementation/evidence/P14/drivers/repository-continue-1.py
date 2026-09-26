"""Resume one explicitly selected repository pair without rewriting live-1.

Uses the original frozen executor/scorer. Run each pair separately so independent
review can happen before the next selection. This is functional quality work;
there is no duration launcher or automatic retry.
"""
import argparse
import copy
import datetime
import json
import os
from pathlib import Path
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[4]
BASE = ROOT / '.local/p14-repository-working-set-preparation'
OLD = ROOT / '.local/p14-repository/live-1'
VERIFY = ROOT / '.local/p14-resume-verification-1'
sys.path[:0] = [str(ROOT / 'tools'), str(BASE)]
from common import verify_original
from release_gate import artifact, fingerprints, input_digest, ref
from release_long_horizon import material
from native_run import run as native_run


def read(path):
    return json.loads(path.read_text())


def write(path, value):
    with path.open('x') as stream:
        json.dump(value, stream, indent=2)
        stream.write('\n')


def inventory(directory):
    rows = []
    for path in sorted(directory.rglob('*')):
        if path.is_symlink():
            raise RuntimeError(f'Symlink in retained matrix: {path}')
        if path.is_file():
            rows.append(ref(path))
    return rows


def inspect(cli):
    result = subprocess.run([str(cli), 'inspect', '/home/alex/Projects/Cruzible'],
                            check=True, capture_output=True, text=True, timeout=60)
    return json.loads(result.stdout)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    parser.add_argument('--pair', required=True,
                        choices=['m1-c1', 'm1-c2', 'm1-c3', 'm2-c0', 'm2-c1', 'm2-c2', 'm2-c3'])
    parser.add_argument('--check-only', action='store_true')
    args = parser.parse_args()
    os.umask(0o077)
    os.chdir(ROOT)
    out = args.output.absolute()
    assert not out.exists() and out.parent == ROOT / '.local/p14-repository'
    start = read(OLD / 'started.json')
    digest = input_digest()
    assert digest == start['input_sha256'] == read(VERIFY / 'identity.json')['input_sha256']
    package = read(ROOT / 'implementation/evidence/P14/package.json')
    artifact(package['archive'])
    assert package['archive']['sha256'] == start['package_sha256']
    for row in start['bound_inputs']:
        assert material(Path(row['path']))['sha256'] == row['sha256']
    stop_path = ROOT / 'implementation/evidence/P14/user-stop-handoff-20260908.json'
    for row in read(stop_path)['artifacts']:
        artifact(row)
    frozen = fingerprints()
    pins = verify_original()
    cli = ROOT / '.local/p14-target/release/ag1-cutover'
    authority = inspect(cli)
    assert authority == read(VERIFY / 'authority-before.json')
    assert authority['authority']['owner'] == 'native' and authority['authority']['quiescent']
    model_index, case_index = (int(part[1:]) for part in args.pair.split('-'))
    source_live = read(ROOT / '.local/p14-long-horizon/live-3/input.json')['live']
    expected_prior_live = copy.deepcopy(source_live)
    expected_prior_live['destination'] = str(OLD / 'm1-c1/ts')
    expected_prior_live['reference']['probe'] = material(BASE / 'worker.mjs')
    expected_prior_live['reference']['materials'] += start['bound_inputs']
    assert expected_prior_live == read(OLD / 'm1-c1/input.json')['live']
    model = source_live['models'][model_index]
    cases = [(row['suite'], case) for row in read(BASE / 'suites.json')
             for case in row['suite']['cases']]
    suite, case = cases[case_index]
    assert model['id'] == start['models'][model_index]
    assert case['id'] == start['cases'][case_index]
    if args.pair == 'm1-c1':
        ts = OLD / 'm1-c1/ts'
        execution = read(ts / 'execution.json')
        assert execution['runtime_settled'] and execution['preservation_verified']
        assert read(ts / 'seed.json')['model'] == model
        assert read(ts / 'case.json') == case
        interrupted = read(OLD / 'm1-c1/rust/cli-settlement.json')
        assert interrupted['exit_code'] == 0 and interrupted['cleanup_error'] is None
        assert all(json.loads(line)['direction'] != 'stdin' for line in
                   (OLD / 'm1-c1/rust/client/protocol.jsonl').read_text().splitlines())
    else:
        assert not (OLD / args.pair).exists()
        ts = out / args.pair / 'ts'
    if args.check_only:
        print(json.dumps({'status': 'passed', 'check_only': True, 'model_actions': 0,
                          'pair': args.pair, 'model': model['id'], 'case': case['id']}))
        return
    assert read(VERIFY / 'audit-execution.json')['exit_code'] == 0
    audit = read(VERIFY / 'full-preservation.json')
    assert audit['status'] == 'production_authority_verified'
    assert audit['original_state_unchanged'] and audit['fixture'] is False
    assert audit['authority'] == authority['authority']
    retained = inventory(OLD)
    out.mkdir(mode=0o700)
    write(out / 'retained-matrix-before.json', retained)
    bindings = [*start['bound_inputs'], material(Path(__file__).resolve())]
    write(out / 'started.json', {
        'utc': datetime.datetime.now(datetime.timezone.utc).isoformat(),
        'fixture': False, 'release_acceptance': False, 'pair': args.pair,
        'input_sha256': digest, 'package_sha256': start['package_sha256'],
        'original_start': ref(OLD / 'started.json'), 'stop_receipt': ref(stop_path),
        'resume_identity': ref(VERIFY / 'identity.json'),
        'preservation_audit': ref(VERIFY / 'full-preservation.json'),
        'retained_inventory': ref(out / 'retained-matrix-before.json'),
        'bound_inputs': bindings, 'model': model['id'], 'case': case['id'],
        'reuse_completed_typescript': args.pair == 'm1-c1',
        'typescript_source': str(ts.relative_to(ROOT)),
        'limitation': 'Interrupted pair spans execution windows; descriptive timing only.',
        'maximum_concurrent_generation': 1, 'automatic_retry': False,
        'duration_workload': False,
    })
    pair = out / args.pair
    pair.mkdir()
    try:
        owner_code = None
        if args.pair != 'm1-c1':
            live = copy.deepcopy(source_live)
            live['destination'] = str(ts)
            live['reference']['probe'] = material(BASE / 'worker.mjs')
            live['reference']['materials'] += bindings
            request = {'live': live, 'model': model['id'], 'workload': {
                'mode': 'original-repository-case', 'suite': suite['name'],
                'case': case['id'], 'generated': material(BASE / 'original-repository-runner.mjs'),
                'suites': material(BASE / 'suites.json'), 'materials': bindings}}
            write(pair / 'input.json', request)
            owner = ROOT / '.local/p14-quality-target/release/repository-owner'
            with (pair / 'owner.log').open('x') as log:
                process = subprocess.run([str(owner), str(pair / 'input.json')],
                                         stdout=log, stderr=subprocess.STDOUT, timeout=14400)
            owner_code = process.returncode
            execution = read(ts / 'execution.json')
            assert execution['runtime_settled'] and execution['preservation_verified']
        ts_result = read(ts / 'typescript-result.json')
        assert ts_result['fixture'] is False
        began = time.monotonic()
        native = native_run(ts, pair / 'rust', source_live, model)
        settlement = read(pair / 'rust/cli-settlement.json')
        assert settlement['exit_code'] == 0 and settlement['cleanup_error'] is None
        write(pair / 'pair-result.json', {
            'fixture': False, 'release_acceptance': False, 'model': model['id'],
            'case': case['id'], 'typescript': ts_result, 'rust': native,
            'typescript_result': ref(ts / 'typescript-result.json'),
            'owner_execution': ref(ts / 'execution.json'),
            'native_result': ref(pair / 'rust/native-result.json'),
            'native_settlement': ref(pair / 'rust/cli-settlement.json'),
            'original_seed': ref(ts / 'seed.json'), 'owner_exit_code': owner_code,
            'native_wrapper_seconds': time.monotonic() - began,
            'claim_review_required': True, 'original_raw_scores_retained': True,
            'typescript_reused_from_interrupted_pair': args.pair == 'm1-c1',
        })
        assert fingerprints() == frozen and verify_original() == pins
        assert inspect(cli) == authority
        print(json.dumps({'pair': args.pair, 'typescript': ts_result['status'],
                          'rust': native['status'], 'review_required': True}), flush=True)
    except BaseException as error:
        write(out / 'interrupted-or-failed.json', {'error': type(error).__name__,
              'message': str(error), 'release_acceptance': False, 'automatic_retry': False})
        raise
    finally:
        after = inventory(OLD)
        write(out / 'retained-matrix-after.json', after)
        assert after == retained, 'Original stopped matrix changed'


if __name__ == '__main__':
    main()
