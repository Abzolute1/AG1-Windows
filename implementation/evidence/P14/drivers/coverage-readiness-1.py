"""Assemble retained P14 requirement coverage without granting release acceptance."""
import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(ROOT / 'tools'))
from release_gate import artifact, input_digest, ref


def read(path):
    return json.loads(path.read_text())


def references(value):
    if isinstance(value, dict):
        if set(value) == {'path', 'sha256'}:
            yield value
        else:
            for child in value.values():
                yield from references(child)
    elif isinstance(value, list):
        for child in value:
            yield from references(child)


def main():
    assert len(sys.argv) == 2
    output = Path(sys.argv[1]).absolute()
    assert output.parent == ROOT / '.local' and output.name.startswith('p14-coverage-readiness-')
    assert not output.exists()
    base = ROOT / '.local/p14-coverage-3'
    mapping = read(base / 'existing-mapping.json')
    providers = read(base / 'provider-mapping.json')
    planning = read(ROOT / 'planning/coverage.json')
    required = {r['id'] for r in planning['capabilities']}
    for group in ('invariants', 'llama_invariants', 'defects'):
        required.update(planning[group])
    assert {r['id'] for r in mapping['rows']} == required
    dispositions = {r['id']: r for r in read(ROOT / 'fixtures/clients/dispositions.json')['rows']}
    provider_rows = {r['id']: r for r in providers['rows']}
    artifacts = {}

    def retain(value):
        for binding in references(value):
            artifact(binding)
            previous = artifacts.setdefault(binding['path'], binding)
            assert previous == binding

    retain(mapping)
    retain(providers)
    rows = []
    for previous in mapping['rows']:
        identity = previous['id']
        evidence = [ref(base / 'existing-mapping.json')]
        evidence += [r['completion'] for r in previous['accepted_phase_coverage']]
        current_tests = [r['test'] for r in previous['named_tests'] if r['current_native_case_passed']]
        disposition = 'Accepted prior implementation scope retained; current native replay and named mappings are separately bound.'
        if identity in provider_rows:
            current_tests += [r['test'] for r in provider_rows[identity]['tests'] if r['current_native_case_passed']]
            evidence.append(ref(base / 'provider-mapping.json'))
            disposition = provider_rows[identity]['scope']
        pending = None
        if identity in ('A76', 'A77'):
            pending = 'P14 live quality, matched performance and full 8h/24h acceptance remain required. Duration launches are explicitly held.'
            evidence += [ref(ROOT / 'implementation/evidence/P14/continue-except-duration-20260908.json'),
                         ref(ROOT / 'implementation/evidence/P14/live5-task-review-observations.json')]
        elif identity == 'I29':
            pending = providers['I29']['pending']
            evidence += providers['I29']['current_observations']
        row = {'id': identity, 'disposition': disposition, 'evidence': evidence,
               'current_named_tests': sorted(set(current_tests)), 'release_accepted': False}
        if identity in dispositions:
            accepted = dispositions[identity]
            assert previous.get('accepted_compatibility_disposition') == accepted
            row.update(compatibility_disposition=accepted['disposition'],
                       compatibility_detail=accepted['detail'])
        if pending:
            row['pending'] = pending
        assert current_tests or previous['accepted_phase_coverage'] or pending, identity
        retain(evidence)
        rows.append(row)

    children = []
    for name in ('provider-storage-children.json', 'tool-children.json', 'extension-client-children.json'):
        source = base / name
        data = read(source)
        retain(data)
        for child in data['rows']:
            cases = []
            for case in child['cases']:
                evidence = [ref(source), *list(references(case))]
                evidence = list({r['path']: r for r in evidence}.values())
                retain(evidence)
                cases.append({'name': case['name'], 'disposition': case['disposition'],
                              'evidence': evidence, 'release_accepted': False})
            children.append({'id': child['id'], 'parent': child['parent'], 'cases': cases})
    inventory = ROOT / 'fixtures/child-inventory.json'
    expected = read(inventory)['children']
    assert len(children) == len(expected) and {r['id'] for r in children} == {r['id'] for r in expected}
    for row in children:
        original = next(r for r in expected if r['id'] == row['id'])
        assert row['parent'] == original['parent']
        assert len(row['cases']) == len(original['cases'])
        assert {r['name'] for r in row['cases']} == set(original['cases'])
    retain([ref(inventory), ref(Path(__file__).resolve()), ref(base / 'differential.json')])
    output.mkdir(mode=0o700)
    result = {'version': 1, 'kind': 'coverage', 'status': 'incomplete', 'release_acceptance': False,
              'input_sha256': input_digest(), 'package_sha256': read(ROOT / 'implementation/evidence/P14/package.json')['archive']['sha256'],
              'rows': rows, 'children': children, 'child_inventory_sha256': ref(inventory)['sha256'],
              'commands': [mapping['fresh_native_command']], 'artifacts': list(artifacts.values()),
              'scope': 'Consolidated exact retained mappings, including all accepted compatibility restrictions. Completeness is not P14 acceptance. Current named checks and historical phase evidence remain distinct; unresolved A76/A77/I29 gates are explicit.'}
    with (output / 'coverage.json').open('x') as stream:
        json.dump(result, stream, indent=2)
        stream.write('\n')
    print(f'{len(rows)} requirements and {len(children)} child families mapped; release acceptance remains incomplete.')


if __name__ == '__main__':
    main()
