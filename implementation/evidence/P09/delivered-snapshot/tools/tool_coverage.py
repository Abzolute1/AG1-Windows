"""Exact P09 reference-case inventory and evidence validation; no inferred coverage."""
from __future__ import annotations

import argparse
import json
from collections import Counter
from pathlib import Path

from common import ROOT, REFERENCE, PIN, GateError, read_json, sha256

CAPABILITIES = ('A16', 'A59', 'A60', 'A61', 'A62', 'A63', 'A64', 'A65', 'A66', 'A68', 'A71')
WINDOWS_SOURCE = 'packages/coding-agent/test/bash-close-hang-windows.test.ts'


def local_file(name: str) -> Path:
    if not isinstance(name, str) or not name or Path(name).is_absolute():
        raise GateError('Coverage evidence requires a workspace-relative file')
    path = (ROOT / name).resolve()
    if not path.is_relative_to(ROOT) or not path.is_file():
        raise GateError(f'Missing or escaping coverage file: {name}')
    return path


def selected_cases() -> list[dict]:
    manifest = read_json(ROOT / 'fixtures/manifest.json')
    reports, cases = {}, {}
    capabilities = [row for row in manifest['capabilities'] if row['id'] in CAPABILITIES]
    if Counter(row['id'] for row in capabilities) != Counter(CAPABILITIES):
        raise GateError('P09 capability inventory changed')
    for capability in capabilities:
        allowed = {}
        for reference in capability['reference_tests']:
            source = reference['path']
            path = (REFERENCE / source).resolve()
            if not path.is_relative_to(REFERENCE) or not path.is_file() or sha256(path) != reference['sha256']:
                raise GateError(f'Stale selected reference test: {source}')
            allowed[source] = reference['sha256']
        if not capability['observed_cases']:
            raise GateError('Empty P09 capability observations')
        for case in capability['observed_cases']:
            report_name = case['report']
            if report_name not in reports:
                reports[report_name] = read_json(local_file(report_name))
            matches = []
            for suite in reports[report_name]['testResults']:
                sources = [source for source in allowed if suite['name'] == source or suite['name'].endswith('/' + source)]
                if len(sources) > 1:
                    raise GateError('Ambiguous selected reference file')
                if sources:
                    for assertion in suite['assertionResults']:
                        if assertion['fullName'] == case['name']:
                            matches.append((sources[0], assertion))
            if len(matches) != 1:
                raise GateError(f'Missing or ambiguous reference case: {case["name"]}')
            source, assertion = matches[0]
            # Manifest normalizes Vitest pending/skipped; the immutable catalog
            # supplies the actual execution status used by the platform exception.
            key = (source, case['name'])
            row = cases.setdefault(key, {
                'path': source, 'name': case['name'], 'sha256': allowed[source],
                'reference_status': assertion['status'], 'capabilities': [],
            })
            if capability['id'] in row['capabilities']:
                raise GateError('Duplicated selected case within a capability')
            row['capabilities'].append(capability['id'])
    return sorted(cases.values(), key=lambda row: (row['path'], row['name']))


def validate_coverage(document: dict, selected: list[dict], passed: set[tuple[str, str]]) -> dict:
    """passed comes from fresh, successful contract execution in the phase gate.

    Names/files are exact. This validates evidence presence, not semantic adequacy:
    reviewers still inspect each case's concrete assertion and referenced test.
    """
    if document.get('version') != 1 or document.get('source_commit') != PIN:
        raise GateError('Unpinned tool coverage document')
    if document.get('capabilities') != list(CAPABILITIES):
        raise GateError('Missing or reordered P09 coverage capabilities')
    expected = {(row['path'], row['name']): row for row in selected}
    if not expected or len(expected) != len(selected):
        raise GateError('Empty or duplicated selected reference inventory')
    rows = document.get('cases')
    if not isinstance(rows, list):
        raise GateError('Missing tool coverage cases')
    seen, counts = set(), Counter()
    for row in rows:
        key = (row.get('path'), row.get('name'))
        if key not in expected or key in seen:
            raise GateError('Unknown or duplicated tool coverage case')
        seen.add(key)
        case = expected[key]
        if row.get('capabilities') != case['capabilities'] or row.get('sha256') != case['sha256']:
            raise GateError('Tool coverage source/capability identity changed')
        status = row.get('status')
        if status == 'platform-pending':
            if case['path'] != WINDOWS_SOURCE or case['reference_status'] not in ('pending', 'skipped'):
                raise GateError('Non-platform behavior cannot be deferred by the coverage gate')
            if row.get('platform') != 'windows' or row.get('gate') != 'P14' or not row.get('reason'):
                raise GateError('Unassigned Windows acceptance case')
        elif status in ('covered', 'corrected'):
            if case['reference_status'] != 'passed':
                raise GateError('Unexecuted reference case cannot be reported as covered')
            evidence = row.get('evidence')
            if not isinstance(evidence, list) or not evidence:
                raise GateError('Case has no native/helper execution evidence')
            for item in evidence:
                path, test = item.get('path'), item.get('test')
                local_file(path)
                if (path, test) not in passed or not isinstance(item.get('assertion'), str) or not item['assertion'].strip():
                    raise GateError('Missing fresh test or concrete assertion for tool case')
            if status == 'corrected':
                correction = row.get('correction', {})
                if not correction.get('reason'):
                    raise GateError('Corrected reference behavior needs an explanation')
                local_file(correction.get('document'))
        else:
            raise GateError(f'Unfinished tool coverage case: {key}: {status}')
        counts[status] += 1
    if seen != set(expected):
        raise GateError(f'Missing tool coverage cases: {len(set(expected) - seen)}')
    return {'selected': len(expected), **dict(sorted(counts.items()))}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--inventory', action='store_true', help='Print selected cases; does not claim coverage')
    args = parser.parse_args()
    if not args.inventory:
        parser.error('Use --inventory; fresh coverage validation is called by the phase gate')
    rows = selected_cases()
    print(json.dumps({'version': 1, 'source_commit': PIN, 'capabilities': list(CAPABILITIES), 'cases': rows}, indent=2))


if __name__ == '__main__':
    main()
