"""P13 cutover regression gate. Fixture evidence cannot certify live acceptance."""
from __future__ import annotations
import datetime
import json
from pathlib import Path
import re
import subprocess

from common import ROOT, GateError, read_json, write_json, sha256, run, verify_original
from client_gate import inputs as client_inputs
from phase_gate import verify_dependencies, verify_artifacts
from runtime_gate import preservation, package_cases
from tool_gate import environment

CASES = {
    'workspace_memory_import_is_cas_bound_to_the_new_store_and_retains_original_bytes',
    'measured_workflow_rejects_wrong_scope_changed_inputs_and_reused_evidence_without_replay',
    'final_authority_audit_requires_quiescence_and_retained_untampered_state',
    'source_snapshot_drift_fences_each_transfer_boundary_without_rolling_back_source',
    'inactive_transfer_requires_verified_shutdown_and_preserves_the_old_owner_on_failure',
    'unaccepted_cutover_can_return_to_unchanged_ts_at_every_boundary_without_losing_archives',
    'runtime_binding_is_archived_and_fixture_config_cannot_launch_the_pinned_backend',
    'unknown_native_execution_metadata_blocks_downgrade_with_lossless_archive_retained',
    'ts_rollback_hides_archived_sessions_and_removes_stale_original_journal_duplicates',
    'administrative_cli_publishes_rollback_and_explicit_native_startup_obeys_ts_authority',
    'bound_configuration_applies_native_settings_and_preserves_updated_vault_on_ts_rollback',
    'drain_requires_both_quiescence_and_released_execution_lease',
    'every_persisted_boundary_reopens_unowned_or_with_exactly_one_owner',
    'duplicate_import_after_restart_is_idempotent_and_commit_cannot_repeat',
    'altered_manifest_configuration_and_source_never_publish_native_authority',
    'authority_tamper_fences_admission_and_cannot_be_silently_overwritten',
    'existing_destination_and_partial_archive_are_never_overwritten',
    'destination_symlink_cannot_redirect_import_into_user_data',
    'copied_isolation_marker_cannot_bypass_transfer_or_open_another_state_writer',
    'migrated_host_resnapshots_full_journal_and_denies_stale_client_and_ts_startup',
    'uncertain_client_effects_survive_rollback_as_receipts_never_session_commands',
    'production_configuration_cannot_use_fixture_acceptance',
    'quiescent_drain_can_be_cancelled_before_any_archive_or_import',
    'registry_rejects_symlinks_and_replaced_workspace_identity',
    'fixture_import_retains_raw_false_zero_order_and_full_source_archive',
    'runtime_binary_library_weight_projector_and_profile_drift_block_import',
    'freeze_must_cover_the_drained_state_not_an_unrelated_valid_bundle',
    'endpoint_aliases_and_competing_hosts_share_one_permit',
    'unclean_typescript_exit_is_not_implicit_permission_to_restart',
    'native_live_endpoint_requires_control_not_just_an_isolated_workspace',
    'abrupt_child_exit_at_each_transfer_stage_is_recoverable',
    'durable_desktop_startup_and_actual_cli_compete_for_the_same_workspace',
    'imported_workspace_runs_prompt_tool_steering_compaction_and_resume',
    'full_native_recovery_retains_private_auxiliary_state_and_never_grants_authority',
    'recovery_rejects_tampered_missing_injected_and_symlink_auxiliary_files',
    'auxiliary_symlinks_and_nested_archive_destinations_block_recovery_export',
    'configuration_import_matches_all_nine_ts_captures_and_retains_private_raw_inputs',
    'configuration_import_rejects_launch_drift_missing_models_and_changed_materials',
    'frozen_configuration_is_self_contained_and_rechecks_original_runtime_before_import',
    'administrative_cli_restores_archive_and_captures_configuration_without_runtime_admission',
    'rollback_restarts_at_each_boundary_and_publishes_exactly_one_new_ts_owner',
    'real_ts_rollback_resnapshots_native_progress_and_inspects_uncertain_effects_without_replay',
    'unsupported_ts_downgrade_and_candidate_tamper_keep_archives_and_native_recovery',
    'rollback_worker_restores_the_selected_branch_without_discarding_other_entries',
    'corrupt_memory_blocks_ts_resume_and_forked_old_cursors_survive_lossless_recovery',
    'abrupt_exit_at_each_rollback_boundary_recovers_without_reopening_stale_ts_state',
}

PENDING = [
    'All three live per-model TS/Rust canaries under exclusive runtime ownership',
    'Production runtime-control transfer with accepted live launch/configuration parity',
    'Production selected-bundle configuration/resource mapping and verified safe TS downgrade disposition',
    'Actual user bundle migration into a new destination and one accepted Rust authority',
    'Complete selected-bundle P13 requirement trace and measured real cutover/resume/rollback evidence',
]

def inputs():
    result = client_inputs()
    for path in list((ROOT/'product/compat/agent-worker').glob('*')) + list((ROOT/'fixtures/runtime').glob('*')):
        if path.is_file(): result[path.relative_to(ROOT).as_posix()] = sha256(path)
    for relative in ['tools/cutover_gate.py', 'tools/cutover_trace.py', 'tools/p13_acceptance.py', 'tools/p13_prepare_runtime.py', 'tools/p13_canaries.py', 'tools/p13_renderer.py', 'tools/p13_prepare_legacy.py', 'tools/probes/p13_runtime_capture.mjs', 'tools/probes/p13_runtime_guard_test.mjs', 'tools/probes/p13_live_reference.mjs', 'planning/coverage.json', 'tools/tests/test_cutover_gate.py', 'phases/P13-authority-cutover.md', 'implementation/P13-HANDOFF.md']:
        result[relative] = sha256(ROOT/relative)
    return result

def validate_contract(log):
    summary = re.findall(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured; (\d+) filtered out;', log)
    names = re.findall(r'^test ([a-zA-Z_][a-zA-Z0-9_]*) \.\.\. ok$', log, re.M)
    if (len(summary) != 1 or int(summary[0][0]) != len(CASES)
            or any(int(n) for n in summary[0][1:]) or len(names) != len(CASES) or set(names) != CASES):
        raise GateError('Incomplete, skipped or duplicated P13 fixture contracts')
    return names

def require_complete(report):
    # This is deliberately separate from the offline regression status.
    if report.get('status') != 'passed' or report.get('pending_gates'):
        raise GateError('P13 mandatory acceptance is pending')
    if report.get('offline_regression_status') != 'passed' or report.get('input_fingerprints',{}).get('product') != inputs():
        raise GateError('P13 requires current-source offline regression evidence')
    verify_artifacts(ROOT, report.get('artifacts', []))
    models = set(read_json(ROOT/'planning/evidence/llama-runtime-lock.json')['models'])
    canaries = report.get('per_model_canaries', [])
    if len(canaries) != len(models) or {row.get('model') for row in canaries} != models:
        raise GateError('P13 needs every configured model canary')
    for row in canaries:
        if row.get('status') != 'passed' or row.get('ownership') != 'exclusive' or row.get('fixture') is not False:
            raise GateError('Fixtures or observations cannot certify live ownership')
    if report.get('authority', {}).get('accepted_rust_cutover') is not True:
        raise GateError('No accepted Rust cutover')
    evidence = report.get('live_canary_evidence')
    if not isinstance(evidence,dict) or evidence not in report.get('artifacts',[]):
        raise GateError('Live acceptance needs retained executable canary evidence')
    verify_artifacts(ROOT,[evidence])
    value=read_json(ROOT/evidence['path'])
    validate_live_canaries(value.get('report',{}),models)
    capture_path=ROOT/'implementation/evidence/P13/runtime-configuration-import.json'
    capture_artifact={'path':capture_path.relative_to(ROOT).as_posix(),'sha256':sha256(capture_path)}
    if capture_artifact not in report.get('artifacts',[]):
        raise GateError('Live evidence must bind the verified runtime configuration')
    validate_live_configuration(value,read_json(capture_path))
    # Reuse the native request builder/quality rules. JSON verification cannot
    # recreate the in-memory runtime capability used by production commit.
    result=subprocess.run([str(ROOT/'product/target/debug/ag1-cutover'),'verify-canaries',str(ROOT/evidence['path'])],
                          capture_output=True,text=True,timeout=60)
    if result.returncode:
        raise GateError('Recorded canary quality/payload verification failed')
    verified=json.loads(result.stdout)
    audit=report.get('authority_evidence')
    if not isinstance(audit,dict) or audit not in report.get('artifacts',[]):
        raise GateError('Local acceptance needs a retained authority audit')
    verify_artifacts(ROOT,[audit])
    value=read_json(ROOT/audit['path'])
    validate_authority_audit(value,verified.get('report_sha256'),read_json(capture_path))
    from p13_acceptance import load_bound, validate_selected
    selected = load_bound(report.get('local_acceptance_evidence'), report['artifacts'])
    if selected.get('authority_evidence') != audit or selected.get('live_canary_evidence') != evidence:
        raise GateError('Selected bundle refers to different authority or canary evidence')
    validate_selected(selected, report, value)
    current=subprocess.run([str(ROOT/'product/target/release/ag1-cutover'),'audit-cutover',value['authority']['workspace']],
                           capture_output=True,text=True,timeout=1800)
    if current.returncode or json.loads(current.stdout)!=value:
        raise GateError('Current authority/state differs from the retained cutover audit')

def validate_authority_audit(value, canary_sha256, capture):
    authority=value.get('authority',{})
    transfer=authority.get('transfer') or {}
    runtime=value.get('runtime_acceptance') or {}
    storage=value.get('storage') or {}
    if (value.get('version')!=1 or value.get('status')!='production_authority_verified'
            or value.get('fixture') is not False or value.get('original_state_unchanged') is not True
            or authority.get('owner')!='native' or authority.get('stage')!='active'
            or authority.get('quiescent') is not True or authority.get('isolated') is not False
            or not authority.get('workspace') or not transfer.get('destination')
            or authority.get('state')!=transfer.get('destination')
            or transfer.get('original_state')==transfer.get('destination')
            or not value.get('selected_session') or value['selected_session']!=transfer.get('session')
            or not value.get('source_snapshot') or storage.get('integrity')!='ok' or storage.get('staged_files')!=0
            or not canary_sha256 or runtime.get('canary_report_sha256')!=canary_sha256
            or runtime.get('capture_sha256')!=capture.get('configuration_sha256')
            or runtime.get('plan_sha256')!=capture.get('plan_sha256')
            or not runtime.get('precommit_recovery') or not runtime.get('recovery_manifest_sha256')):
        raise GateError('Authority audit does not bind a preserved production cutover to these live canaries')

def validate_live_configuration(value, capture):
    if (capture.get('fixture') is not False or capture.get('status')!='offline_configuration_verified'
            or value.get('configuration',{}).get('sha256')!=capture.get('configuration_sha256')
            or value.get('configuration',{}).get('plan_sha256')!=capture.get('plan_sha256')):
        raise GateError('Live matrix does not match the verified runtime capture')
    models=value.get('models',[])
    expected={m['model']:(m['context_window'],m['max_tokens']) for m in capture.get('models',[])}
    if (len(models)!=3 or len(expected)!=3 or {m.get('id') for m in models}!=set(expected)
            or any(m.get('provider')!='llamacpp' or m.get('api')!='openai-completions'
                or (m.get('contextWindow'),m.get('maxTokens'))!=expected.get(m.get('id')) for m in models)):
        raise GateError('Canary catalog changes a preserved model or context/output limit')
    expected_events=[(model,action) for model in value.get('report',{}).get('models',[])
                     for _ in range(2) for action in ['owned_start']+['owned_ready']*6+['owned_stop']]
    events=value.get('runtime_events',[])
    if (len(expected_events)!=48 or [(e.get('model'),e.get('action')) for e in events]!=expected_events
            or any(e.get('plan_sha256')!=capture['plan_sha256'].get(e.get('model'))
                   or type(e.get('elapsed_ms')) is not int or e['elapsed_ms']<0 for e in events)):
        raise GateError('Missing ordered owned launch/readiness/stop evidence for each adapter')

def validate_live_canaries(value, models):
    required=['exact-copy','reasoning','tool-call','tool-result','context-limit','cancellation']
    if (value.get('version')!=1 or value.get('fixture') is not False or value.get('status')!='passed'
            or value.get('failures') or value.get('runtime_settled') is not True
            or value.get('cold_start_per_runner') is not True
            or len(value.get('models',[]))!=3 or set(value['models'])!=set(models)):
        raise GateError('Incomplete live canary matrix')
    expected=[(model,runner,case) for model in value['models'] for runner in ['typescript','rust'] for case in required]
    rows=value.get('rows',[])
    if [(row.get('model'),row.get('runner'),row.get('case')) for row in rows]!=expected:
        raise GateError('Canaries must execute sequentially for every model and adapter')
    for row in rows:
        answer=row.get('answer',{})
        if (answer.get('settled') is not True or answer.get('payload',{}).get('model')!=row['model']
                or not answer.get('runtime_before')
                or (row['case']!='cancellation' and not answer.get('runtime_after'))
                or answer.get('cancelled') is not (row['case']=='cancellation')
                or (row['case']!='context-limit' and answer.get('first_content_ms') is None)):
            raise GateError('Missing transport settlement, timing or runtime ownership samples')

def window():
    process = subprocess.run(['ps', '-C', 'llama-server', '-o', 'pid,ppid,lstart,args'], capture_output=True, text=True, timeout=10)
    if process.returncode not in (0, 1):
        raise GateError('Cannot inspect the host runtime window')
    gpu = subprocess.run(['nvidia-smi', '--query-gpu=name,memory.total,memory.used,memory.free', '--format=csv,noheader'], capture_output=True, text=True, timeout=10)
    if gpu.returncode:
        raise GateError('Cannot inspect GPU availability')
    return {'observed_at': datetime.datetime.now(datetime.timezone.utc).isoformat(),
            'processes': process.stdout, 'gpu': gpu.stdout,
            'process_ownership_granted': False, 'model_actions': 0,
            'status': 'external_processes_present' if process.returncode == 0 else 'no_process_observed_not_an_ownership_grant'}

def execute_p13():
    evidence = ROOT/'implementation/evidence/P13'
    evidence.mkdir(parents=True, exist_ok=True)
    commands = []
    _, env = environment()
    def execute(name, argv):
        print('P13: '+name, flush=True)
        log = evidence/('offline-'+name+'.log')
        commands.append(run(argv, env=env, log=log, timeout=1800))
        return log.read_text()
    completion = evidence/'completion.json'
    write_json(completion, {'phase': 'P13', 'status': 'running', 'dependencies': ['P12']})
    try:
        for phase in read_json(ROOT/'planning/phase-graph.json'):
            if phase == 'P14': break
            verify_dependencies(phase)
        original = verify_original()
        preserved = preservation()
        write_json(evidence/'runtime-window-resumed-before.json', window())
        execute('license-capture', ['python3', 'tools/record_protocol_licenses.py'])
        before = inputs()
        execute('format', ['cargo', 'fmt', '--manifest-path', 'product/Cargo.toml', '--all', '--', '--check'])
        execute('clippy', ['cargo', 'clippy', '--offline', '--manifest-path', 'product/Cargo.toml', '-p', 'ag1-platform', '-p', 'ag1-cli', '-p', 'ag1-desktop', '--all-targets', '--features', 'ag1-desktop/visual-test', '--', '-D', 'warnings'])
        validators = execute('validator-tests', ['python3', '-m', 'unittest', 'discover', '-s', 'tools/tests', '-p', 'test_cutover_gate.py', '-v'])
        if not re.search(r'\nRan [1-9]\d* tests in [^\n]+\n\nOK\n', validators): raise GateError('Empty gate validator selection')
        execute('frontend-typecheck', ['npm', '--prefix', 'product/apps/desktop', 'run', 'typecheck'])
        execute('frontend-tests', ['npm', '--prefix', 'product/apps/desktop', 'test', '--', '--run', '--reporter=json', '--outputFile='+str(evidence/'frontend-tests.json')])
        from reference_oracles import validate_vitest
        frontend = read_json(evidence/'frontend-tests.json')
        validate_vitest(frontend, expected=frontend.get('numTotalTests',0))
        if frontend.get('numTotalTests',0) < 15: raise GateError('Incomplete current frontend contracts')
        execute('frontend-build', ['npm', '--prefix', 'product/apps/desktop', 'run', 'build'])
        execute('cli-build', ['cargo', 'build', '--offline', '--manifest-path', 'product/Cargo.toml', '-p', 'ag1-cli', '--bins'])
        execute('audit-release-build', ['cargo', 'build', '--offline', '--release', '--manifest-path', 'product/Cargo.toml', '-p', 'ag1-cli', '--bin', 'ag1-cutover'])
        execute('renderer-build', ['cargo', 'build', '--offline', '--manifest-path', 'product/Cargo.toml', '-p', 'ag1-desktop', '-p', 'ag1-cli', '--bins', '--examples', '--features', 'ag1-desktop/visual-test'])
        import secrets
        renderer_output = ROOT/'.local'/('p13-renderer-gate-'+secrets.token_hex(8))
        execute('renderer-rehearsal', ['python3', 'tools/p13_renderer.py', str(renderer_output), '--rehearse'])
        from p13_renderer import validate as validate_renderer
        from p13_acceptance import fingerprint
        renderer = read_json(renderer_output/'observation.json')
        validate_renderer(renderer, True)
        if renderer.get('input_sha256') != fingerprint(before):
            raise GateError('Renderer rehearsal does not match current inputs')
        renderer_refs = [{'path':str(renderer_output/'observation.json'),'sha256':sha256(renderer_output/'observation.json')},
                         renderer['screenshot'], renderer['started_receipt'], renderer['finished_receipt']]
        verify_artifacts(ROOT, renderer_refs)
        write_json(evidence/'renderer-rehearsal.json', {'status':'passed','fixture':True,'phase_acceptance':False,
            'freeze_ms':renderer['freeze']['elapsed_ms'],'resnapshot_ms':renderer['elapsed_ms'],
            'native_observations':len(renderer['observations']),'evidence':renderer_refs})
        cases = validate_contract(execute('cutover-contract', ['cargo', 'test', '--offline', '--manifest-path', 'product/Cargo.toml', '-p', 'ag1-desktop', '--test', 'cutover_contract']))
        regressions = set()
        for package in ['ag1-core', 'ag1-storage', 'ag1-protocol', 'ag1-platform', 'ag1-cli']:
            log = execute(package, ['cargo', 'test', '--offline', '--manifest-path', 'product/Cargo.toml', '-p', package, '--all-targets'])
            folder = ROOT/('product/clients/cli/tests' if package == 'ag1-cli' else f'product/crates/{package}/tests')
            regressions.update(package_cases(package, log, [p.stem for p in folder.glob('*.rs')]))
        desktop = execute('bridge-contract', ['cargo', 'test', '--offline', '--manifest-path', 'product/Cargo.toml', '-p', 'ag1-desktop', '--test', 'bridge_contract'])
        from provider_gate import validate_rust_log
        validate_rust_log(desktop, 3)
        execute('protocol', ['cargo', 'run', '--offline', '--manifest-path', 'product/Cargo.toml', '-p', 'ag1-protocol', '--bin', 'protocol-codegen', '--', '--check'])
        if original != verify_original() or preserved != preservation() or before != inputs():
            raise GateError('Original source, runtime or current inputs changed during gate')
        for phase in read_json(ROOT/'planning/phase-graph.json'):
            if phase == 'P14': break
            verify_dependencies(phase)
        write_json(evidence/'runtime-window-resumed-after.json', window())
        write_json(evidence/'runtime-preservation-resumed.json', {'before':preserved, 'after':preservation(), 'original_pins_unchanged':True})
        from cutover_trace import inventory
        write_json(evidence/'requirement-trace.json', inventory(cases))
        write_json(evidence/'offline-contracts.json', {'status':'passed', 'cutover_cases':cases, 'native_regression_cases':len(regressions), 'phase_acceptance':False})
        artifacts = [{'path':p.relative_to(ROOT).as_posix(),'sha256':sha256(p)} for p in sorted(evidence.rglob('*')) if p.is_file() and p != completion]
        from p13_acceptance import private_artifacts
        artifacts.extend(renderer_refs)
        artifacts.extend(private_artifacts())
        verify_artifacts(ROOT, artifacts)
        report = {'phase':'P13', 'status':'blocked', 'observed_at':datetime.datetime.now(datetime.timezone.utc).isoformat(),
                  'dependencies':['P12'], 'input_fingerprints':{'product':before}, 'artifacts':artifacts, 'commands':commands,
                  'coverage':{'fixture_cutover_contracts':len(cases), 'native_regression_cases':len(regressions), 'live_canaries':0},
                  'offline_regression_status':'passed', 'authority':{'before':'Ordinary TS desktop; pre-existing external tuned model service',
                      'after':'Original user state and services unchanged; fixture migrations only', 'accepted_rust_cutover':False},
                  'per_model_canaries':[{'model':model,'status':'pending','fixture':False,'ownership':'not_granted'} for model in read_json(ROOT/'planning/evidence/llama-runtime-lock.json')['models']],
                  'pending_gates':PENDING, 'profile_writes':0, 'live_model_actions':0, 'P14_started':False}
        from p13_acceptance import assemble
        report = assemble(report)
        if report['status'] == 'passed':
            require_complete(report)
        write_json(completion, report)
    except Exception as error:
        write_json(completion, {'phase':'P13','status':'failed','dependencies':['P12'],'commands':commands,'error':str(error),'pending_gates':PENDING})
        raise
    if report['status'] != 'passed':
        require_complete(report)
