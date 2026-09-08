"""Release acceptance requires measured, fingerprinted reports; absence never passes."""
from __future__ import annotations

import datetime
import hashlib
import json
import math
import platform
import re
import shutil
import subprocess
import uuid
from pathlib import Path

from common import ROOT, GateError, read_json, run, sha256, write_json

EVIDENCE = ROOT / 'implementation/evidence/P14'
REPORTS = ('live', 'duration-8h', 'duration-24h', 'performance',
           'platform-linux', 'platform-windows', 'platform-macos', 'signing', 'coverage', 'provenance', 'runtime-preservation')
SCENARIOS = {'source-edit', 'branch', 'fork', 'resume', 'detach-reconnect',
             'model-crash', 'host-crash', 'corrupt-state-recovery'}


def fingerprints():
    from cutover_gate import inputs
    from context_gate import inputs as context_inputs
    from runtime_differential import inputs as runtime_inputs
    result = inputs()
    result.update(context_inputs())
    result.update(runtime_inputs())
    for folder in ('product/release', 'fixtures/release', 'tools/release_duration'):
        for file in (ROOT / folder).rglob('*'):
            if file.is_file() and '__pycache__' not in file.parts:
                result[file.relative_to(ROOT).as_posix()] = sha256(file)
    for pattern in ('release*.py', 'release*.mjs', 'tests/test_release*.py'):
        for file in (ROOT / 'tools').glob(pattern):
            result[file.relative_to(ROOT).as_posix()] = sha256(file)
    for name in ('tools/reference_oracles.py', 'phases/P14-release-and-long-horizon-gates.md'):
        result[name] = sha256(ROOT / name)
    return result


def input_digest():
    return hashlib.sha256(json.dumps(fingerprints(), sort_keys=True, separators=(',', ':')).encode()).hexdigest()


def accepted_canary_digest(cli):
    accepted = read_json(ROOT/'implementation/evidence/P13/local-acceptance.json')['live_canary_evidence']
    path = Path(accepted['path'])
    if sha256(path) != accepted['sha256']:
        raise GateError('Accepted canary artifact changed')
    checked = subprocess.run([str(cli),'verify-canaries',str(path)], capture_output=True, text=True, timeout=60)
    if checked.returncode:
        raise GateError('Accepted native canary verifier refused its retained recording')
    digest = json.loads(checked.stdout).get('report_sha256')
    if not isinstance(digest,str) or not re.fullmatch('[0-9a-f]{64}',digest):
        raise GateError('Native canary report digest missing')
    return digest


def artifact(row, root=ROOT):
    if not isinstance(row, dict) or set(row) != {'path', 'sha256'}:
        raise GateError('Expected an exact path/SHA-256 artifact reference')
    path = root / row['path']
    if Path(row['path']).is_absolute() or '..' in Path(row['path']).parts:
        raise GateError('Artifact escapes evidence root')
    if not path.resolve().is_relative_to(root.resolve()) or any(p.is_symlink() for p in [path, *path.parents] if p != root.parent):
        raise GateError('Symlinked artifact is not evidence')
    if not path.is_file() or sha256(path) != row['sha256']:
        raise GateError('Missing or stale artifact: ' + row['path'])
    return path


def ref(path):
    return {'path': path.relative_to(ROOT).as_posix(), 'sha256': sha256(path)}


def number(value, name, minimum=0):
    if type(value) not in (int, float) or not math.isfinite(value) or value < minimum:
        raise GateError('Invalid measurement: ' + name)
    return value


def rust_results(log, minimum=1):
    summaries = re.findall(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured; (\d+) filtered out;', log)
    passed = sum(int(row[0]) for row in summaries)
    cases = re.findall(r'^test (.+) \.\.\. ok$', log, re.M)
    if not summaries or passed < minimum or passed != len(cases) or any(int(n) for row in summaries for n in row[1:]):
        raise GateError('Empty, failed, skipped or incomplete native regression run')
    return {'passed': passed, 'cases': cases}


def validate_duration(data, hours):
    start = number(data.get('start_monotonic_ns'), 'start', 1)
    end = number(data.get('end_monotonic_ns'), 'end', start)
    if (end - start) / 1e9 < hours * 3600:
        raise GateError(f'A shorter workload cannot satisfy {hours}h')
    policy = data.get('activity_policy', {})
    idle = number(policy.get('max_idle_seconds'), 'activity gap budget', 1)
    activity_limit = number(policy.get('max_activity_seconds'), 'activity duration budget', 1)
    if idle > 120 or activity_limit > 1800 or policy.get('minimum_tasks_per_hour') != 2:
        raise GateError('Duration must keep bounded repeated work active throughout the clock')
    activities = data.get('activities', [])
    if sum(a.get('kind') == 'task' for a in activities) < hours * 2:
        raise GateError('At least two completed agent tasks per hour are required')
    cursor, seen = start, set()
    for activity in activities:
        identity = activity.get('id')
        if not isinstance(identity, str) or not identity or identity in seen:
            raise GateError('Missing or duplicate workload activity identity')
        seen.add(identity)
        if activity.get('kind') not in SCENARIOS | {'task', 'admission', 'token-calibration', 'checkpoint-verifier'} or activity.get('passed') is not True or not activity.get('finished_receipt'):
            raise GateError('Unexecuted or unverified workload activity')
        begin = number(activity.get('started_monotonic_ns'), 'activity start', cursor)
        finish = number(activity.get('finished_monotonic_ns'), 'activity finish', begin + 1)
        if begin - cursor > idle * 1e9 or finish - begin > activity_limit * 1e9 or finish > end:
            raise GateError('Idle padding, overlapping work or unbounded activity cannot satisfy duration')
        cursor = finish
    if end - cursor > idle * 1e9:
        raise GateError('Sleeping after a short workload cannot satisfy duration')
    tokens = number(data.get('externalized_tokens'), 'externalized tokens', 2_000_000)
    if tokens > 3_000_000 or data.get('token_accounting') != 'externalized-content-once':
        raise GateError('Require 2–3M distinct externalized tokens, not repeated token counting')
    rows = data.get('sessions', [])
    if len(rows) < 2 or {case.get('scenario') for case in rows} != SCENARIOS:
        raise GateError('Missing repeated-session/fault scenarios')
    for case in rows:
        if case.get('executed') is not True or case.get('passed') is not True or not case.get('evidence'):
            raise GateError('Unexecuted duration scenario')
    samples = data.get('samples', [])
    if len(samples) < hours * 60:
        raise GateError('Missing minute-level duration memory observations')
    times = [number(r.get('monotonic_ns'), 'sample time', start) for r in samples]
    if times != sorted(set(times)) or times[-1] > end or times[0] - start > 120e9 or end - times[-1] > 120e9 or any(b-a > 120e9 for a,b in zip(times, times[1:])):
        raise GateError('Duration sampling gaps or invalid clock domain')
    warmup = number(data.get('warmup_seconds'), 'warmup', 60)
    if warmup > 3600:
        raise GateError('Warmup cannot exclude the measured workload')
    selected = [s for s in samples if (s['monotonic_ns']-start)/1e9 >= warmup]
    xs = [(s['monotonic_ns']-start)/1e9 for s in selected]
    ys = [number(s.get('heap_bytes'), 'heap') for s in selected]
    for s in samples:
        for key in ('heap_bytes', 'pss_bytes', 'persistent_evidence_bytes', 'owned_children'):
            number(s.get(key), key)
    meanx, meany = sum(xs)/len(xs), sum(ys)/len(ys)
    slope = sum((x-meanx)*(y-meany) for x,y in zip(xs,ys))/sum((x-meanx)**2 for x in xs)
    # The budget must have been frozen in the workload start receipt, not selected after measurement.
    limit = number(data.get('heap_growth_budget_bytes_per_hour'), 'memory budget')
    if slope * 3600 > limit:
        raise GateError('Post-warmup heap growth exceeds the recorded budget')
    from release_duration.memory_analysis import analyze
    try:
        lifetime_analysis = analyze(samples, start, warmup, limit)
    except (ValueError, KeyError, TypeError) as error:
        raise GateError('Invalid native lifetime memory observations: '+str(error)) from error
    if lifetime_analysis['status'] != 'passed':
        raise GateError('Native lifetime growth cannot be hidden by restarts or missing host samples')
    quality(data)
    return {'duration_seconds': (end-start)/1e9, 'heap_slope_bytes_per_hour': slope*3600, 'native_lifetimes':lifetime_analysis}


def quality(data):
    q = data.get('quality', {})
    for key in ('required_recall', 'forbidden_avoidance', 'contradiction_handling', 'task_success'):
        row = q.get(key, {})
        if type(row.get('total')) is not int or row['total'] <= 0 or row.get('passed') != row['total']:
            raise GateError('Missing or failed quality verifier: ' + key)
    for key in ('stale_authoritative_claims', 'leaked_owned_children', 'lost_commands', 'replayed_effects', 'runtime_preservation_violations'):
        if type(q.get(key)) is not int or q[key] != 0:
            raise GateError('Missing or failed safety observation: ' + key)


def percentile(samples):
    if not isinstance(samples, list) or not samples:
        raise GateError('Empty performance samples')
    values = sorted(number(s, 'sample') for s in samples)
    return values[math.ceil(len(values)*.95)-1]


def validate_performance(data):
    if data.get('all_shell_processes_included') is not True or data.get('matched_conditions') is not True:
        raise GateError('Unmatched or incomplete shell process accounting')
    if not data.get('process_inventory') or not data.get('external_dependencies') or not data.get('cache_conditions'):
        raise GateError('Missing process, optional runtime or cache inventory')
    for key in ('shell_ready_ms', 'preservation_admission_ms', 'model_load_ms', 'usable_session_ready_ms'):
        if len(data.get(key, [])) < 20:
            raise GateError('Twenty separate startup milestones required: ' + key)
        percentile(data[key])
    if percentile(data['shell_ready_ms']) >= 1500 or percentile(data.get('display_lag_ms', [])) >= 50:
        raise GateError('Packaged startup/display latency budget failed')
    if number(data.get('idle_seconds'), 'idle duration') < 300 or number(data.get('idle_cpu_core_percent'), 'idle CPU') >= .5:
        raise GateError('Five-minute idle CPU budget failed')
    for native, old, ratio in (('shell_pss_bytes', 'dyad_pss_bytes', .7), ('install_bytes', 'dyad_install_bytes', .5)):
        if number(data.get(native), native, 1) > number(data.get(old), old, 1)*ratio:
            raise GateError('Matched packaged reduction failed: ' + native)
    for key in ('paused_subscriber_max_bytes', 'paused_subscriber_budget_bytes'):
        number(data.get(key), key, 1)
    if data['paused_subscriber_max_bytes'] > data['paused_subscriber_budget_bytes']:
        raise GateError('Paused subscriber exceeded queue budget')
    core = data.get('core', [])
    if {row.get('operation') for row in core} != {'context', 'retrieval', 'persistence'}:
        raise GateError('Missing equivalent core measurements')
    for row in core:
        if len(row.get('alternating_runs', [])) < 3 or not row.get('noise_analysis'):
            raise GateError('Missing repeated/noise-characterized core comparison')
        for pair in row['alternating_runs']:
            if pair.get('rust_input_sha256') != pair.get('ts_input_sha256') or not pair.get('rust_input_sha256'):
                raise GateError('Core benchmark inputs differ')
            if len(pair.get('rust_ms', [])) < 20 or len(pair.get('ts_ms', [])) < 20 or percentile(pair['rust_ms']) > percentile(pair['ts_ms'])*1.1:
                raise GateError('Core p95 regressed more than 10%')
    quality(data)


def verify_allocator_samples(samples, inventory, root):
    import xml.etree.ElementTree as ET
    recordings = {}
    for sample in samples:
        if not sample.get('host_present'):
            if sample.get('heap_observation') is not None or sample['heap_bytes'] != 0:
                raise GateError('Absent native host has a fabricated allocator sample')
            continue
        heap = sample['heap_observation']
        try:
            path = Path(heap['raw_file'])
            relative = path.relative_to(root).as_posix()
            if relative not in inventory:
                raise GateError('Raw native allocator recording is absent from evidence inventory')
            if relative not in recordings:
                artifact({'path':relative,'sha256':inventory[relative]}, root)
                rows = [json.loads(line) for line in path.read_text().splitlines()]
                recordings[relative] = {r['monotonic_ns']:r for r in rows}
                if len(rows) != len(recordings[relative]):
                    raise GateError('Duplicate allocator observation time')
            raw = recordings[relative][heap['monotonic_ns']]
            if raw['pid'] != heap['pid'] or raw['malloc_info_result'] != 0:
                raise GateError('Raw allocator process identity or sampling result differs')
            xml = ET.fromstring(raw['xml'])
            totals = {r.attrib['type']:int(r.attrib['size']) for r in xml.findall('total')}
            reserved = int(xml.find("system[@type='current']").attrib['size']) + totals['mmap']
            estimate = reserved - totals['rest'] - totals.get('fast',0)
            if estimate != sample['heap_bytes'] or estimate != heap['heap_bytes'] or reserved != heap['allocator_reserved_bytes'] or len(xml.findall('heap')) != heap['arenas']:
                raise GateError('Reported native heap differs from raw allocator XML')
        except (ValueError,KeyError,TypeError,AttributeError,ET.ParseError) as error:
            raise GateError('Invalid raw allocator evidence: '+str(error)) from error


def validate_report(kind, data, digest, package_sha, root=ROOT):
    if data.get('version') != 1 or data.get('kind') != kind or data.get('status') != 'passed' or data.get('input_sha256') != digest or data.get('package_sha256') != package_sha:
        raise GateError('Failed, stale or incorrectly scoped release report: ' + kind)
    if not data.get('artifacts') or not data.get('commands'):
        raise GateError('Reports require retained raw evidence and execution commands')
    paths = [artifact(r, root) for r in data['artifacts']]
    if len(set(paths)) != len(paths):
        raise GateError('Duplicate raw evidence')
    inventory = {r['path']: r['sha256'] for r in data['artifacts']}
    for cmd in data['commands']:
        if not cmd.get('argv') or type(cmd.get('exit_code')) is not int or cmd['exit_code'] != 0 or cmd.get('log') not in inventory:
            raise GateError('Report requires actual successful commands and retained logs')
    if kind.startswith('duration-'):
        validate_duration(data, 8 if kind == 'duration-8h' else 24)
        sample_file = artifact(data.get('memory_samples'), root)
        if inventory.get(data['memory_samples']['path']) != data['memory_samples']['sha256'] or [json.loads(line) for line in sample_file.read_text().splitlines()] != data['samples']:
            raise GateError('Duration samples differ from the retained minute observation log')
        verify_allocator_samples(data['samples'], inventory, root)
        analysis = read_json(artifact(data.get('memory_analysis'), root))
        from release_duration.memory_analysis import analyze
        if analysis != analyze(data['samples'],data['start_monotonic_ns'],data['warmup_seconds'],data['heap_growth_budget_bytes_per_hour']):
            raise GateError('Retained memory fit differs from observed native lifetimes')
        start = read_json(artifact(data.get('start_receipt'), root))
        if start.get('input_sha256') != digest or start.get('package_sha256') != package_sha or start.get('start_monotonic_ns') != data.get('start_monotonic_ns') or start.get('heap_growth_budget_bytes_per_hour') != data.get('heap_growth_budget_bytes_per_hour') or start.get('activity_policy') != data.get('activity_policy') or start.get('warmup_seconds') != data.get('warmup_seconds'):
            raise GateError('Duration does not bind its predeclared workload receipt')
        for activity in data['activities']:
            receipt = activity['finished_receipt']
            if inventory.get(receipt['path']) != receipt['sha256']:
                raise GateError('Workload activity receipt absent from report inventory')
            executed = read_json(artifact(receipt, root))
            if executed.get('status') != 'passed' or any(executed.get(k) != activity.get(k) for k in ('id', 'kind', 'started_monotonic_ns', 'finished_monotonic_ns')):
                raise GateError('Activity differs from retained execution receipt')
            evidence = executed.get('evidence', [])
            if not evidence or any(inventory.get(r['path']) != r['sha256'] for r in evidence):
                raise GateError('Activity lacks retained command/trace/verifier evidence')
        for case in data['sessions']:
            for r in case['evidence']:
                if inventory.get(r['path']) != r['sha256']:
                    raise GateError('Scenario evidence is absent from report inventory')
    elif kind == 'performance':
        validate_performance(data)
    elif kind == 'live':
        quality(data)
        if data.get('fixture') is not False or data.get('exclusive_runtime_lease') is not True or data.get('maximum_concurrent_generation') != 1:
            raise GateError('Live release evaluation requires exclusive authenticated runtime custody')
        capture = read_json(ROOT / 'implementation/evidence/P13/runtime-configuration-import.json')
        expected = capture['plan_sha256']
        pairs = data.get('pairs', [])
        if {p.get('model') for p in pairs} != set(expected):
            raise GateError('Missing configured live model')
        scenarios = {'long-horizon-context', 'repository-context', 'context-intelligence', 'speed-probe'}
        if len(pairs) != len(expected)*len(scenarios) or {(p.get('model'),p.get('scenario')) for p in pairs} != {(m,s) for m in expected for s in scenarios}:
            raise GateError('Missing or duplicate matched live scenarios')
        for pair in pairs:
            if pair.get('plan_sha256') != expected[pair['model']] or pair.get('adapters') != ['typescript','rust'] or pair.get('passed') is not True:
                raise GateError('Live pair changed the preserved launch plan or failed')
            for key in ('assembled_prompts', 'context', 'cache_conditions', 'metrics', 'verifier'):
                if not pair.get(key):
                    raise GateError('Missing retained live comparison input/output: ' + key)
                for r in pair[key]:
                    if inventory.get(r['path']) != r['sha256']:
                        raise GateError('Live evidence absent from inventory')
    elif kind.startswith('platform-'):
        expected = {'shell', 'process', 'protocol', 'preview', 'interrupted-update', 'runtime-path-preservation', 'crash-startup', 'unknown-pending-operation', 'rollback-archive'}
        rows = data.get('cases', [])
        if data.get('os') != kind.removeprefix('platform-') or len(rows) != len(expected) or {r.get('name') for r in rows} != expected or any(r.get('executed') is not True or r.get('passed') is not True for r in rows):
            raise GateError('Missing executed packaged platform cases')
    elif kind == 'signing':
        if {r.get('os') for r in data.get('signatures', [])} != {'linux','windows','macos'} or any(r.get('verified') is not True for r in data['signatures']):
            raise GateError('Missing verified per-OS release signatures')
    elif kind == 'runtime-preservation':
        from cutover_gate import validate_authority_audit
        capture = read_json(ROOT / 'implementation/evidence/P13/runtime-configuration-import.json')
        validate_authority_audit(read_json(artifact(data.get('authority_audit'),root)),
                                 accepted_canary_digest(ROOT/'.local/p14-target/release/ag1-cutover'), capture)
    elif kind == 'provenance':
        for key in ('actual_binary_closure_reviewed','all_redistributed_notices_retained',
                    'no_pro_code_types_assets','no_dyad_production_telemetry_or_branding',
                    'no_private_models_or_credentials','external_runtime_write_targets_excluded'):
            if data.get(key) is not True:
                raise GateError('Missing release provenance review: '+key)
    elif kind == 'coverage':
        planning = read_json(ROOT / 'planning/coverage.json')
        required = {r['id'] for r in planning['capabilities']}
        for group in ('invariants','llama_invariants','defects'):
            required.update(planning[group])
        rows = data.get('rows', [])
        if not required or len(rows) != len(required) or {r.get('id') for r in rows} != required:
            raise GateError('Incomplete final parent/invariant/defect/llama coverage')
        for row in rows:
            if not row.get('disposition') or not row.get('evidence'):
                raise GateError('Coverage needs actual evidence and retained dispositions')
            for r in row['evidence']:
                if inventory.get(r['path']) != r['sha256']:
                    raise GateError('Coverage evidence absent from retained inventory')
        dispositions = {r['id']:r['disposition'] for r in read_json(ROOT/'fixtures/clients/dispositions.json')['rows']}
        for row in rows:
            if row['id'] in dispositions and row.get('compatibility_disposition') != dispositions[row['id']]:
                raise GateError('Release changed an accepted capability disposition')
        children = read_json(ROOT / 'fixtures/child-inventory.json')
        expected_children = {r['id']:r for r in children['children']}
        delivered_children = data.get('children', [])
        if data.get('child_inventory_sha256') != sha256(ROOT / 'fixtures/child-inventory.json') or len(delivered_children) != len(expected_children) or {r.get('id') for r in delivered_children} != set(expected_children):
            raise GateError('Missing final child requirement evidence')
        for row in delivered_children:
            expected = expected_children[row['id']]
            cases = row.get('cases', [])
            if row.get('parent') != expected['parent'] or len(cases) != len(expected['cases']) or {c.get('name') for c in cases} != set(expected['cases']):
                raise GateError('Missing final child case evidence')
            for case in cases:
                if not case.get('disposition') or not case.get('evidence'):
                    raise GateError('Missing child case disposition/evidence')
                for r in case['evidence']:
                    if inventory.get(r['path']) != r['sha256']:
                        raise GateError('Child evidence absent from retained inventory')


def prerequisites():
    return {'observed_at': datetime.datetime.now(datetime.timezone.utc).isoformat(),
            'os': platform.system(), 'machine': platform.machine(),
            'free_disk_bytes': shutil.disk_usage(ROOT).free,
            'installed_rust_targets': subprocess.check_output(['rustup','target','list','--installed'], text=True).splitlines(),
            'tools': {name: shutil.which(name) for name in ('dpkg-deb','rpmbuild','codesign','osslsigncode','gh','node')},
            'windows_runner': 'not configured', 'macos_runner': 'not configured',
            'signing': 'release identities/credentials not configured',
            'required_workload_hours': [8,24],
            'local_inference_platforms': {'linux':'preserved builds; fresh lease/audit required', 'windows':'unavailable', 'macos':'unavailable'}}


def execute_p14():
    from phase_gate import verify_dependencies
    from tool_gate import environment
    EVIDENCE.mkdir(parents=True, exist_ok=True)
    completion = EVIDENCE / 'completion.json'
    commands, pending = [], []
    write_json(completion, {'phase':'P14','status':'running','pending_gates':list(REPORTS)})
    try:
        for phase in read_json(ROOT / 'planning/phase-graph.json'):
            verify_dependencies(phase)
        p13 = read_json(ROOT / 'implementation/evidence/P13/completion.json')
        if p13.get('pending_gates') or p13.get('status') != 'passed':
            raise GateError('P13 acceptance is incomplete')
        before = fingerprints()
        node, env = environment()
        env['CARGO_TARGET_DIR'] = str(ROOT / '.local/p14-target')
        logs = EVIDENCE / 'checks' / uuid.uuid4().hex
        commands.append(run(['python3','-m','unittest','discover','-s','tools/tests','-p','test_release*.py','-v'], log=logs/'validators.log'))
        commands.append(run(['cargo','fmt','--manifest-path','product/Cargo.toml','--all','--','--check'], env=env, log=logs/'format.log'))
        commands.append(run(['cargo','test','--offline','--locked','--manifest-path','product/Cargo.toml','-p','ag1-platform','--example','release-quality'], env=env, log=logs/'quality-driver-tests.log',timeout=1800))
        rust_results((logs/'quality-driver-tests.log').read_text(),minimum=3)
        commands.append(run(['npm','--prefix','product/apps/desktop','run','build'], env=env, log=logs/'frontend.log'))
        commands.append(run(['npm','--prefix','product/apps/desktop','test','--','run'], env=env, log=logs/'frontend-tests.log'))
        commands.append(run(['python3','tools/reference_oracles.py','--suite','release-differential','--offline'], env=env, log=logs/'differential.log', timeout=7200))
        from release_package import select_candidate
        packaged = select_candidate()
        if fingerprints() != before:
            raise GateError('Release inputs changed during checks or packaging')
        write_json(EVIDENCE / 'prerequisites.json', prerequisites())
        digest = input_digest()
        accepted, partial_observations = {}, {}
        for kind in REPORTS:
            path = EVIDENCE / 'reports' / (kind + '.json')
            if not path.is_file():
                if kind == 'live':
                    observed = EVIDENCE / 'context-intelligence.json'
                    if observed.is_file():
                        partial_observations['context-intelligence'] = ref(observed)
                        if read_json(observed).get('status') in ('failed', 'failed-quality'):
                            pending.append('live: recorded context-intelligence failure; full matched live report is also absent')
                            continue
                pending.append(kind + ': required measured report is absent')
                continue
            value = read_json(path)
            validate_report(kind, value, digest, packaged['archive']['sha256'])
            accepted[kind] = ref(path)
        result = {'phase':'P14','status':'incomplete' if pending else 'passed',
                  'completed_at':datetime.datetime.now(datetime.timezone.utc).isoformat(),
                  'dependencies':['P13'],'input_fingerprints':fingerprints(),
                  'commands':commands,'reports':accepted,'package':packaged,
                  'partial_observations':partial_observations,
                  'coverage':{'scope':['A76','A77'],'release_acceptance':not pending},
                  'authority':{'before':'accepted native owner','after':'unchanged; no migration or runtime control'},
                  'pending_gates':pending}
        # Top-level driver logs can still be receiving this process's stdout.
        # Hash the completed command logs below checks/, never our own live sink.
        result['artifacts'] = [ref(p) for p in sorted(EVIDENCE.rglob('*')) if p.is_file() and p != completion
                               and not (p.parent == EVIDENCE and p.suffix == '.log')]
        result['artifacts'].extend(packaged['artifacts'])
        write_json(completion, result)
        print(json.dumps({'phase':'P14','status':result['status'],'pending_gates':pending}))
        if pending:
            raise GateError('P14 incomplete: required release measurements remain pending')
    except Exception as exc:
        existing = read_json(completion)
        if existing.get('status') != 'incomplete':
            write_json(completion, {'phase':'P14','status':'failed','commands':commands,'error':str(exc),'pending_gates':pending or list(REPORTS)})
        raise
