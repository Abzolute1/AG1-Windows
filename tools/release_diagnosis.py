#!/usr/bin/env python3
"""Predeclared transport diagnosis; never replaces the retained failed quality run."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import time

from common import ROOT, GateError, read_json, run, sha256, write_json
from release_gate import EVIDENCE, artifact, fingerprints, ref
from tool_gate import environment


def main(output):
    os.umask(0o077)
    base = ROOT / '.local/p14-quality'
    output = output.absolute()
    if base.is_symlink() or output.exists() or output.parent.resolve() != base.resolve():
        raise GateError('Use a new direct child of .local/p14-quality')
    failed = read_json(EVIDENCE / 'context-intelligence.json')
    if failed['status'] != 'failed-quality':
        raise GateError('Diagnosis requires the retained failed run')
    for row in failed['artifacts']:
        artifact(row)
    started_path = next(artifact(r) for r in failed['artifacts'] if r['path'].endswith('/started.json'))
    started = read_json(started_path)
    original_input = artifact(started['input'])
    historical = original_input.parent
    request = read_json(original_input)
    case_id = 'revision-aware-memory:baseline'
    model = 'Qwen3.8-27B-UD-Q4_K_S'
    request['cases'] = [c for c in request['cases'] if c['id'] == case_id]
    if len(request['cases']) != 1:
        raise GateError('Missing exact failed prompt')
    request['diagnostic'] = {'model': model, 'repetitions': 4}
    request['live']['destination'] = str(output / 'matrix')
    # This inspection is not custody: the native driver must still acquire the
    # shared lease, reject external GPU users and freshly verify all materials.
    cli = ROOT / '.local/p14-target/release/ag1-cutover'
    inspected = subprocess.run([str(cli), 'inspect', '/home/alex/Projects/Cruzible'],
                               capture_output=True, text=True, check=True, timeout=60)
    authority = json.loads(inspected.stdout)['authority']
    previous = read_json(EVIDENCE / 'final-preservation-inspection.json')['authority']['authority']
    if authority != previous or not authority['quiescent'] or authority['owner'] != 'native':
        raise GateError('Authority changed since retained quiescent inspection')
    output.mkdir(mode=0o700)
    write_json(output / 'input.json', request)
    before = fingerprints()
    binary = ROOT / '.local/p14-quality-target/release/examples/release-quality'
    command = [str(binary), str(output / 'input.json')]
    write_json(output / 'started.json', {'argv': command, 'executable': ref(binary),
        'input': ref(output / 'input.json'), 'historical_failure': ref(EVIDENCE / 'context-intelligence.json'),
        'input_fingerprints': before, 'authority': authority, 'repetitions': 4,
        'order': ['typescript,rust', 'rust,typescript', 'typescript,rust', 'rust,typescript'],
        'sampling_changes': False, 'automatic_retry': False, 'release_acceptance': False})
    start = time.monotonic()
    with (output / 'native.log').open('x') as log:
        completed = subprocess.run(command, stdout=log, stderr=subprocess.STDOUT, timeout=14400)
    write_json(output / 'finished.json', {'exit_code': completed.returncode,
        'seconds': time.monotonic() - start, 'started': ref(output / 'started.json')})
    if completed.returncode:
        raise GateError('Diagnostic execution failed; private evidence retained')
    node, env = environment()
    score = run([str(node), 'tools/release_diagnosis.mjs', str(output), str(historical)],
                env=env, log=output / 'score.log')
    rows = [json.loads(line) for line in (output / 'matrix/rows.jsonl').read_text().splitlines()]
    execution = read_json(output / 'matrix/execution.json')
    if len(rows) != 8 or execution['status'] != 'executed' or not execution['runtime_settled']:
        raise GateError('Incomplete diagnostic pair/settlement')
    wires = []
    for index, row in enumerate(rows, 1):
        if row['runner'] != 'rust':
            continue
        path = output / f'matrix/wire-row-{index}.sse'
        data = path.read_bytes()
        metadata = read_json(path.with_suffix('.json'))
        if not metadata['eof'] or sum(metadata['chunk_lengths']) != len(data):
            raise GateError('Missing transport EOF or incomplete byte capture')
        text, finish, done = '', [], False
        for event in data.decode('utf-8').replace('\r\n', '\n').split('\n\n'):
            payload = '\n'.join(line[5:].lstrip(' ') for line in event.splitlines() if line.startswith('data:'))
            if not payload:
                continue
            if payload == '[DONE]':
                done = True
                continue
            value = json.loads(payload)
            for choice in value.get('choices', []):
                if choice.get('index', 0) != 0:
                    continue
                text += choice.get('delta', {}).get('content') or ''
                if choice.get('finish_reason'):
                    finish.append(choice['finish_reason'])
        decoded = ''.join(b['text'] for b in row['answer']['message']['content'] if b['type'] == 'text')
        try:
            json.loads(text)
            valid_json = True
        except json.JSONDecodeError:
            valid_json = False
        wires.append({'trial': row['trial'], 'raw_content_matches_adapter': text == decoded,
            'raw_content_valid_json': valid_json, 'finish_reasons': finish, 'done': done, 'wire': ref(path)})
    report = {'status': 'diagnosed', 'release_acceptance': False,
        'scope': 'Four predeclared alternating pairs of the exact failed prompt and model. Rust response-body capture precedes decoding. This cannot retroactively identify the uncaptured original failure or establish full quality parity.',
        'historical_failure': ref(EVIDENCE / 'context-intelligence.json'),
        'quality': read_json(output / 'scores.json'), 'wire_observations': wires,
        'input_fingerprints': before, 'inputs_unchanged': fingerprints() == before,
        'commands': [{'argv': command, 'exit_code': completed.returncode, 'log': str((output / 'native.log').relative_to(ROOT))}, score],
        'artifacts': [ref(p) for p in sorted(output.rglob('*')) if p.is_file()]}
    write_json(EVIDENCE / 'transport-diagnosis.json', report)
    print(json.dumps({'quality': report['quality'], 'wire_observations': wires}, indent=2))
    if not report['inputs_unchanged'] or any(not w['raw_content_matches_adapter'] for w in wires):
        raise GateError('Diagnostic source changed or adapter differs from captured body')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('new_private_output', type=Path)
    main(parser.parse_args().new_private_output)
