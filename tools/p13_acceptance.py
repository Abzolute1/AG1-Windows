"""Bind selected-bundle evidence to the current P13 regression checkpoint.

This module performs no migration or model action. Missing evidence keeps the
phase pending; invalid supplied evidence fails the gate. Reports do not mint the
native in-memory runtime capability needed to commit production authority.
"""
from __future__ import annotations
import copy
import hashlib
import json
import os
from pathlib import Path

from common import ROOT, GateError, read_json, sha256
from phase_gate import verify_artifacts

ACCEPTANCE = 'implementation/evidence/P13/local-acceptance.json'
REQUIREMENTS = ({f'A{i:02}' for i in range(1, 86)} | {f'I{i:02}' for i in range(1, 28)}
                | {f'L{i:02}' for i in range(1, 9)} | {f'B{i:02}' for i in range(1, 5)})
WORKFLOW = {'prompt', 'tool', 'compaction', 'steering', 'resume',
            'stale-client-rejection', 'renderer-resnapshot'}
MAPPING = {'journals', 'cursors', 'blobs', 'authorization-roots', 'settings',
           'credentials', 'resources', 'pending-decisions', 'receipts', 'source-git'}
TIMINGS = {'drain', 'import', 'resume', 'recovery', 'renderer-resnapshot'}


def private_artifacts(root=ROOT):
    """Retain explicit private evidence hashes without publishing its contents."""
    path = root/ACCEPTANCE
    if not path.exists():
        return []
    refs = read_json(path).get('private_artifacts', [])
    if not isinstance(refs, list) or len(refs) > 1024:
        raise GateError('Invalid private evidence inventory')
    for ref in refs:
        if not isinstance(ref, dict) or set(ref) != {'path', 'sha256'}:
            raise GateError('Invalid private evidence reference')
        path = root/ref['path']
        if not path.resolve().is_relative_to((root/'.local').resolve()) or path.suffix not in {'.json', '.png'}:
            raise GateError('Private cutover observations must be JSON or PNG under .local')
        if any(parent.is_symlink() for parent in [path, *path.parents]):
            raise GateError('Private evidence must not follow symlinks')
        stat = path.stat()
        if stat.st_uid != os.geteuid() or stat.st_mode & 0o077 or stat.st_size > 128*1024*1024:
            raise GateError('Private cutover observations require private, bounded files')
    verify_artifacts(root, refs)
    return refs


def fingerprint(inputs):
    return hashlib.sha256(json.dumps(inputs, sort_keys=True, separators=(',', ':')).encode()).hexdigest()


def load_bound(reference, artifacts, root=ROOT):
    if (not isinstance(reference, dict) or set(reference) != {'path', 'sha256'}
            or reference not in artifacts):
        raise GateError('Selected-bundle evidence must be in the retained artifact manifest')
    verify_artifacts(root, [reference])
    path = root/reference['path']
    if path.stat().st_size > 128 * 1024 * 1024:
        raise GateError('Selected-bundle report exceeds the bounded evidence size')
    value = read_json(path)
    if not isinstance(value, dict):
        raise GateError('Selected-bundle report must be an object')
    return value


def scope(value, selection, source):
    if (value.get('version') != 1 or value.get('status') != 'passed'
            or value.get('fixture') is not False or value.get('selection') != selection
            or value.get('input_sha256') != source):
        raise GateError('Fixture, stale or unrelated selected-bundle evidence')


def rows_by_id(rows, required):
    if (not isinstance(rows, list) or len(rows) != len(required)
            or any(not isinstance(row, dict) for row in rows)
            or {row.get('id') for row in rows} != required):
        raise GateError('Incomplete or duplicated selected-bundle evidence rows')
    return {row['id']: row for row in rows}


def validate_selected(value, report, audit, root=ROOT):
    """Validate mappings, the executed workflow and measured recovery closure."""
    authority = audit['authority']
    transfer = authority['transfer']
    selection = {'workspace': authority['workspace'], 'session': transfer['session'],
                 'original_state': transfer['original_state'], 'destination': transfer['destination']}
    source = fingerprint(report['input_fingerprints']['product'])
    scope(value, selection, source)
    if value.get('source_snapshot') != audit.get('source_snapshot'):
        raise GateError('Selected-bundle source snapshot differs from the authority audit')
    artifacts = report['artifacts']
    dispositions_path = root/'fixtures/clients/dispositions.json'
    if value.get('compatibility_sha256') != sha256(dispositions_path):
        raise GateError('Selected trace must preserve the pinned client compatibility dispositions')
    dispositions = {row['id']: row['disposition'] for row in read_json(dispositions_path)['rows']}

    def evidence(row):
        refs = row.get('evidence', [])
        if not refs or len({ref.get('path') for ref in refs if isinstance(ref, dict)}) != len(refs):
            raise GateError('Selected-bundle row lacks distinct executed evidence')
        outputs = []
        for ref in refs:
            output = load_bound(ref, artifacts, root)
            scope(output, selection, source)
            outputs.append(output)
        return outputs

    mappings = rows_by_id(value.get('mapping'), MAPPING)
    for row in mappings.values():
        if (row.get('disposition') not in {'imported', 'preserved-reference', 'absent'}
                or not isinstance(row.get('reason'), str) or not row['reason'].strip()):
            raise GateError('Unresolved selected-bundle configuration/resource mapping')
        evidence(row)
    trace = rows_by_id(value.get('requirements'), REQUIREMENTS)
    for row in trace.values():
        if (row.get('disposition') not in {'verified', 'reference-only', 'unsupported'}
                or not isinstance(row.get('reason'), str) or not row['reason'].strip()):
            raise GateError('Unresolved selected-bundle requirement disposition')
        # Core invariants and corrected defects cannot be waived as unsupported.
        if not row['id'].startswith('A') and row['disposition'] != 'verified':
            raise GateError('Mandatory invariant or corrected defect cannot be waived')
        if row['id'].startswith('A'):
            expected = ('reference-only' if row['id'] in {'A80', 'A81', 'A82', 'A83', 'A84'}
                        else 'unsupported' if row['id'] == 'A85' else 'verified')
            if row['disposition'] != expected or row.get('compatibility_disposition') != dispositions[row['id']]:
                raise GateError('Selected trace changes the existing client capability boundary')
        evidence(row)

    workflow = rows_by_id(value.get('workflow'), WORKFLOW)
    for row in workflow.values():
        outputs = evidence(row)
        if not any(output.get('case') == row['id'] and output.get('executed') is True
                   and isinstance(output.get('observations'), list) and output['observations']
                   for output in outputs):
            raise GateError('Workflow row lacks executed selected-session observations')
        if row['id'] == 'renderer-resnapshot':
            from p13_renderer import validate as validate_renderer
            rendered = [output for output in outputs if output.get('case') == 'renderer-resnapshot']
            for output in rendered:
                validate_renderer(output, False)
                if output['screenshot'] not in artifacts:
                    raise GateError('Renderer screenshot is not retained in the artifact manifest')
                verify_artifacts(root, [output['screenshot']])
    counters = value.get('safety', {})
    for key, expected in {'max_workspace_writers': 1, 'max_generations': 1,
                          'replayed_external_mutations': 0, 'lost_accepted_commands': 0,
                          'original_data_writes': 0, 'profile_writes': 0}.items():
        if type(counters.get(key)) is not int or counters[key] != expected:
            raise GateError('Cutover violated writer, receipt or preservation requirements')
    stops = counters.get('external_service_stops')
    authorized = counters.get('authorized_service_stops', [])
    if type(stops) is not int or stops < 0 or not isinstance(authorized, list) or stops != len(authorized):
        raise GateError('External service stops require explicit verified authorization evidence')
    seen_stops = set()
    for row in authorized:
        outputs = evidence(row)
        for output in outputs:
            if (output.get('case') != 'authorized-service-stop' or output.get('user_authorized') is not True
                    or output.get('identity_verified') is not True or output.get('exited') is not True
                    or type(output.get('pid')) is not int or output['pid'] in seen_stops):
                raise GateError('Unverified or repeated external service stop')
            seen_stops.add(output['pid'])
            start = load_bound(output.get('started_receipt'), artifacts, root)
            finish = load_bound(output.get('finished_receipt'), artifacts, root)
            if (start.get('pid') != output['pid'] or finish.get('pid') != output['pid']
                    or not start.get('authorization') or not start.get('exe') or not start.get('stat', start.get('start_ticks'))
                    or finish.get('graceful_exit_observed') is not True or finish.get('forced') is not False):
                raise GateError('Missing identity, authorization or graceful service-exit receipts')
    if len(seen_stops) != stops:
        raise GateError('External stop count differs from retained receipts')
    evidence(counters)

    timings = rows_by_id(value.get('measurements'), TIMINGS)
    for row in timings.values():
        if type(row.get('elapsed_ms')) is not int or row['elapsed_ms'] < 0:
            raise GateError('Missing measured cutover duration')
        outputs = evidence(row)
        if not any(output.get('operation') == row['id']
                   and output.get('elapsed_ms') == row['elapsed_ms']
                   and output.get('clock') == 'monotonic' and output.get('executed') is True
                   and output.get('started_receipt') and output.get('finished_receipt')
                   for output in outputs):
            raise GateError('Duration must bind an executed operation and its receipts')
        for output in outputs:
            if output.get('operation') != row['id']:
                continue
            start = load_bound(output['started_receipt'], artifacts, root)
            finish = load_bound(output['finished_receipt'], artifacts, root)
            if (finish.get('started_sha256') != output['started_receipt']['sha256']
                    or finish.get('status') != 'command_succeeded'
                    or finish.get('elapsed_ms') != row['elapsed_ms']
                    or start.get('status') != 'started'
                    or start.get('automatic_retry_allowed') is not False
                    or not start.get('arguments') or not start.get('executable_sha256')):
                raise GateError('Missing, failed or mismatched durable measurement receipts')
            # An administrative transition after host shutdown is not drain time.
            if row['id'] == 'drain' and start['arguments'][0] == 'begin-transfer':
                raise GateError('Administrative begin-transfer does not measure host draining')
    recovery = value.get('recovery', {})
    if (recovery.get('status') != 'passed' or recovery.get('original_reopened') is not False
            or recovery.get('archives_retained') is not True
            or recovery.get('compatible_ts_resume') is not True
            or not recovery.get('new_destination')
            or recovery['new_destination'] in {selection['original_state'], selection['destination']}):
        raise GateError('Recovery must verify compatible resume in a new destination and retain archives')
    evidence(recovery)


def assemble(report, root=ROOT):
    """Return a candidate, never publish it before native reinspection succeeds."""
    path = root/ACCEPTANCE
    if not path.exists():
        return report
    if report.get('offline_regression_status') != 'passed':
        raise GateError('Local acceptance requires a passed current-source regression checkpoint')
    reference = {'path': ACCEPTANCE, 'sha256': sha256(path)}
    value = load_bound(reference, report['artifacts'], root)
    authority_ref = value.get('authority_evidence')
    audit = load_bound(authority_ref, report['artifacts'], root)
    # Native verification and the production audit run again in require_complete.
    from cutover_gate import validate_authority_audit
    capture = load_bound(value.get('configuration_evidence'), report['artifacts'], root)
    canaries = load_bound(value.get('live_canary_evidence'), report['artifacts'], root)
    from cutover_gate import validate_live_canaries, validate_live_configuration
    models = set(read_json(root/'planning/evidence/llama-runtime-lock.json')['models'])
    validate_live_canaries(canaries.get('report', {}), models)
    validate_live_configuration(canaries, capture)
    report_hash = audit.get('runtime_acceptance', {}).get('canary_report_sha256')
    validate_authority_audit(audit, report_hash, capture)
    validate_selected(value, report, audit, root)
    candidate = copy.deepcopy(report)
    candidate.update(status='passed', pending_gates=[], local_acceptance_evidence=reference,
                     authority_evidence=authority_ref, live_canary_evidence=value['live_canary_evidence'],
                     authority={'before': 'TypeScript selected workspace', 'after': audit['authority'],
                                'accepted_rust_cutover': True},
                     per_model_canaries=[{'model': model, 'status': 'passed', 'fixture': False,
                                          'ownership': 'exclusive'} for model in sorted(models)],
                     live_model_actions='Retained sequential live runtime events')
    candidate['coverage']['live_canaries'] = len(canaries['report']['rows'])
    return candidate
