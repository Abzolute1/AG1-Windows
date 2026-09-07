"""P13 trace inventory. Inherited evidence is distinct from migrated acceptance."""
from common import ROOT, GateError, read_json, sha256

TRACE = {
    'workspace-owner': ('A01 A03 I01 I14 I15', [
        'final_authority_audit_requires_quiescence_and_retained_untampered_state',
        'durable_desktop_startup_and_actual_cli_compete_for_the_same_workspace',
        'copied_isolation_marker_cannot_bypass_transfer_or_open_another_state_writer',
        'administrative_cli_publishes_rollback_and_explicit_native_startup_obeys_ts_authority']),
    'drain-and-recovery': ('A04 A05 A06 A08 A09 A17 A18 I03 I13 I18', [
        'measured_workflow_rejects_wrong_scope_changed_inputs_and_reused_evidence_without_replay',
        'source_snapshot_drift_fences_each_transfer_boundary_without_rolling_back_source',
        'drain_requires_both_quiescence_and_released_execution_lease',
        'imported_workspace_runs_prompt_tool_steering_compaction_and_resume',
        'abrupt_child_exit_at_each_transfer_stage_is_recoverable',
        'abrupt_exit_at_each_rollback_boundary_recovers_without_reopening_stale_ts_state']),
    'receipt-custody': ('A61 A62 A63 A75 I04 I15 I19 I27 B01', [
        'real_ts_rollback_resnapshots_native_progress_and_inspects_uncertain_effects_without_replay',
        'uncertain_client_effects_survive_rollback_as_receipts_never_session_commands',
        'full_native_recovery_retains_private_auxiliary_state_and_never_grants_authority']),
    'memory-and-artifacts': ('A38 A40 A41 A42 A43 A44 A45 I10 I11 I12 B02 B04', [
        'corrupt_memory_blocks_ts_resume_and_forked_old_cursors_survive_lossless_recovery',
        'recovery_rejects_tampered_missing_injected_and_symlink_auxiliary_files']),
    'journal-and-downgrade': ('A46 A47 A48 I05', [
        'rollback_worker_restores_the_selected_branch_without_discarding_other_entries',
        'ts_rollback_hides_archived_sessions_and_removes_stale_original_journal_duplicates',
        'unknown_native_execution_metadata_blocks_downgrade_with_lossless_archive_retained',
        'unsupported_ts_downgrade_and_candidate_tamper_keep_archives_and_native_recovery']),
    'private-configuration': ('A49 A51 A54 A55 L01 L02 L07', [
        'runtime_binding_is_archived_and_fixture_config_cannot_launch_the_pinned_backend',
        'bound_configuration_applies_native_settings_and_preserves_updated_vault_on_ts_rollback',
        'configuration_import_matches_all_nine_ts_captures_and_retains_private_raw_inputs',
        'configuration_import_rejects_launch_drift_missing_models_and_changed_materials',
        'frozen_configuration_is_self_contained_and_rechecks_original_runtime_before_import']),
    'runtime-preservation': ('A52 A53 A57 I16 I21 L03 L04 L05 L06 L08', [
        'endpoint_aliases_and_competing_hosts_share_one_permit',
        'native_live_endpoint_requires_control_not_just_an_isolated_workspace',
        'runtime_binary_library_weight_projector_and_profile_drift_block_import',
        'production_configuration_cannot_use_fixture_acceptance']),
}

def inventory(passed):
    plan = read_json(ROOT/'planning/coverage.json')
    dispositions = read_json(ROOT/'fixtures/clients/dispositions.json')
    required = {f'A{i:02}' for i in range(1,86)} | {f'I{i:02}' for i in range(1,28)} | {f'L{i:02}' for i in range(1,9)} | {f'B{i:02}' for i in range(1,5)}
    groups = []
    for name, (ids, tests) in TRACE.items():
        if not set(tests) <= set(passed): raise GateError('Unexecuted P13 trace: '+name)
        groups.append({'name':name,'requirements':ids.split(),'status':'fixture_contracts_passed',
                       'tests':[{'path':'product/apps/desktop/src-tauri/tests/cutover_contract.rs','test':test} for test in tests],
                       'production_acceptance':False})
    rows = []
    for key in sorted(required):
        if key.startswith('A'):
            source = next(row for row in plan['capabilities'] if row['id']==key)
            phases = [source['phase']]
            disposition = next(row for row in dispositions['rows'] if row['id']==key)
        elif key.startswith('B'):
            source = plan['defects'][key]
            phases = [source['fix']]
            disposition = source['oracle']
        else:
            phases = plan['llama_invariants' if key.startswith('L') else 'invariants'][key]
            disposition = 'Preserve the existing invariant; fixture evidence does not certify live cutover.'
        inherited = []
        for phase in phases:
            if int(phase[1:]) >= 13: continue
            path = ROOT/f'implementation/evidence/{phase}/completion.json'
            if read_json(path).get('status') != 'passed': raise GateError('Unpassed inherited trace '+key)
            inherited.append({'path':path.relative_to(ROOT).as_posix(),'sha256':sha256(path)})
        rows.append({'id':key,'inherited_completion':inherited,'disposition':disposition,
                     'p13_fixture_groups':[g['name'] for g in groups if key in g['requirements']],
                     'selected_user_bundle_acceptance':'pending'})
    return {'phase':'P13','status':'incomplete','scope':'85 capabilities, I01–I27, L01–L08 and B01–B04',
            'rows':rows,'fixture_groups':groups,
            'limits':['Unlisted integration coverage remains inherited, not newly certified.',
                      'Native background jobs, subagent checkpoints/runs and unknown execution metadata block TS downgrade until explicitly mapped.',
                      'Production runtime custody, all three live canaries and selected user-bundle measurements remain mandatory.']}
