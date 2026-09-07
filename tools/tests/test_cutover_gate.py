import copy
import sys
import unittest
import json
import os
import subprocess
import tempfile
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from common import GateError, ROOT, read_json
from cutover_gate import CASES, validate_contract, require_complete, validate_live_canaries, validate_live_configuration, validate_authority_audit

class CutoverGateTests(unittest.TestCase):
    def test_legacy_inventory_rejects_wrong_workspace_missing_session_and_links_without_writes(self):
        from p13_prepare_legacy import inventory
        with tempfile.TemporaryDirectory() as folder:
            root=Path(folder);workspace=root/'workspace';source=root/'state';workspace.mkdir();source.mkdir()
            path=source/'session.jsonl'
            raw=json.dumps({'type':'session','id':'selected','cwd':str(workspace),'version':3})+'\n'
            path.write_text(raw)
            manifest, report=inventory(workspace,source,'selected')
            self.assertEqual(manifest['files'][0]['kind'],'journal');self.assertEqual(report['original_data_writes'],0)
            for target,session in [(root,'selected'),(workspace,'missing')]:
                with self.assertRaises(GateError):inventory(target,source,session)
            link=source/'link';link.symlink_to(path)
            with self.assertRaises(GateError):inventory(workspace,source,'selected')
            self.assertEqual(path.read_text(),raw)

    def renderer_observation(self, selection):
        scope = {'session':selection['session'],'workspace':'fixture','instance':'fixture','epoch':'1'}
        snapshot = {'scope':scope,'workspace_path':selection['workspace'],'fixture_provider':False,
                    'status':'idle','revision':'1'}
        return {'version':1,'status':'passed','fixture':False,'case':'renderer-resnapshot','selection':selection,
                'executed':True,'renderer':'owned-tauri-webkit','dom_marker_visible':True,'tools_restored':True,
                'desktop_exit_code':0,'authority_quiescent':True,'elapsed_ms':7,'freeze':{'elapsed_ms':30000},
                'screenshot':{'path':'synthetic.png','sha256':'0'*64},
                'before':{'snapshot':snapshot},'after':{'snapshot':{**snapshot,'revision':'2'}},
                'observations':[{'scope':scope,'status':'streaming','since_arm_ms':1000,'error':None},
                                {'scope':scope,'status':'idle','since_arm_ms':2000,'marker_seen':True,'error':None}]}

    def test_renderer_requires_independent_completion_during_freeze_and_same_session(self):
        from p13_renderer import validate
        value = self.renderer_observation({'workspace':'/fixture','session':'selected'})
        validate(value, False)
        for key, new in [('fixture',True),('renderer','native-client'),('freeze',{'elapsed_ms':29999}),
                         ('dom_marker_visible',False),('tools_restored',False),('authority_quiescent',False),
                         ('observations',[]),('elapsed_ms',True),('screenshot',{})]:
            changed=copy.deepcopy(value);changed[key]=new
            with self.assertRaises(GateError):validate(changed,False)
        for edit in range(6):
            changed=copy.deepcopy(value)
            if edit==0:changed['observations'][1]['since_arm_ms']=31000
            if edit==1:changed['observations'].reverse()
            if edit==2:changed['observations'][1]['marker_seen']=False
            if edit==3:changed['after']['snapshot']['scope']={'session':'unrelated'}
            if edit==4:changed['observations'][0]['error']='recovery_required'
            if edit==5:changed['observations'][0]['status']='idle'
            with self.assertRaises(GateError):validate(changed,False)

    def test_private_observations_are_pinned_but_never_follow_links_or_escape_private_storage(self):
        from p13_acceptance import private_artifacts, ACCEPTANCE
        from common import sha256
        with tempfile.TemporaryDirectory() as folder:
            root=Path(folder)
            private=root/'.local';private.mkdir(mode=0o700)
            observation=private/'observation.json';observation.write_text('{"fixture":true}')
            observation.chmod(0o600)
            path=root/ACCEPTANCE;path.parent.mkdir(parents=True)
            reference={'path':str(observation),'sha256':sha256(observation)}
            def inventory(ref):path.write_text(json.dumps({'private_artifacts':[ref]}))
            inventory(reference)
            self.assertEqual(private_artifacts(root),[reference])
            observation.chmod(0o644)
            with self.assertRaises(GateError):private_artifacts(root)
            observation.chmod(0o600)
            link=private/'linked.json';link.symlink_to(observation)
            inventory({'path':str(link),'sha256':reference['sha256']})
            with self.assertRaises(GateError):private_artifacts(root)
            outside=root/'outside.json';outside.write_bytes(observation.read_bytes());outside.chmod(0o600)
            inventory({'path':str(outside),'sha256':reference['sha256']})
            with self.assertRaises(GateError):private_artifacts(root)
            inventory(reference);observation.write_text('{}')
            with self.assertRaises(GateError):private_artifacts(root)

    def test_acceptance_assembly_never_promotes_missing_evidence_or_failed_offline_checks(self):
        from p13_acceptance import assemble, ACCEPTANCE
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            pending = {'status':'blocked', 'offline_regression_status':'passed'}
            self.assertEqual(assemble(pending, root), pending)
            path = root/ACCEPTANCE
            path.parent.mkdir(parents=True)
            path.write_text('{"status":"passed","fixture":false}')
            with self.assertRaises(GateError):
                assemble({'status':'failed','offline_regression_status':'failed'}, root)
            with self.assertRaises(GateError):
                assemble({**pending,'artifacts':[]}, root)

    def selected_acceptance(self, root):
        from p13_acceptance import fingerprint, REQUIREMENTS, MAPPING, WORKFLOW, TIMINGS
        from common import sha256
        selection = {'workspace':'/selected/workspace','session':'selected',
                     'original_state':'/selected/original','destination':'/selected/native'}
        report = {'input_fingerprints':{'product':{'fixture.rs':'0'*64}},'artifacts':[]}
        dispositions_path = root/'fixtures/clients/dispositions.json'
        dispositions_path.parent.mkdir(parents=True)
        dispositions_path.write_bytes((ROOT/'fixtures/clients/dispositions.json').read_bytes())
        dispositions = {row['id']:row['disposition'] for row in json.loads(dispositions_path.read_text())['rows']}
        base = {'version':1,'fixture':False,'status':'passed','selection':selection,
                'input_sha256':fingerprint(report['input_fingerprints']['product'])}
        def retain(name, value):
            path = root/(name+'.json'); path.write_text(json.dumps(value))
            ref = {'path':path.name,'sha256':sha256(path)}
            report['artifacts'].append(ref)
            return ref
        observed = retain('observations',{**base,'observations':['Synthetic validator input only']})
        value = {**base, 'source_snapshot':{'sha256':'1'*64},'compatibility_sha256':sha256(dispositions_path),
            'mapping':[{'id':key,'disposition':'imported','reason':'Fixture validator mapping','evidence':[observed]} for key in sorted(MAPPING)],
            'requirements':[{'id':key,'disposition':'verified','reason':'Fixture validator trace','evidence':[observed]} for key in sorted(REQUIREMENTS)],
            'workflow':[], 'measurements':[],
            'safety':{'max_workspace_writers':1,'max_generations':1,'replayed_external_mutations':0,
                      'lost_accepted_commands':0,'original_data_writes':0,'profile_writes':0,
                      'external_service_stops':0,'evidence':[observed]},
            'recovery':{'status':'passed','original_reopened':False,'archives_retained':True,
                        'compatible_ts_resume':True,'new_destination':'/selected/recovery','evidence':[observed]}}
        for row in value['requirements']:
            if row['id'].startswith('A'):
                row['compatibility_disposition'] = dispositions[row['id']]
                if row['id'] in {'A80','A81','A82','A83','A84'}: row['disposition']='reference-only'
                if row['id']=='A85': row['disposition']='unsupported'
        for case in sorted(WORKFLOW):
            observation = {**base,'case':case,'executed':True,'observations':['Synthetic validator input only']}
            if case == 'renderer-resnapshot':
                observation.update(self.renderer_observation(selection))
                screenshot = root/'synthetic.png'
                screenshot.write_bytes(b'Synthetic validator screenshot, not execution evidence')
                observation['screenshot'] = {'path':screenshot.name,'sha256':sha256(screenshot)}
                report['artifacts'].append(observation['screenshot'])
            ref = retain(case,observation)
            value['workflow'].append({'id':case,'evidence':[ref]})
        for operation in sorted(TIMINGS):
            start = retain(operation+'-start',{'status':'started','arguments':[operation],
                           'automatic_retry_allowed':False,'executable_sha256':'2'*64})
            finish = retain(operation+'-finish',{'status':'command_succeeded','started_sha256':start['sha256'],'elapsed_ms':7})
            ref = retain(operation+'-timing',{**base,'operation':operation,'elapsed_ms':7,
                         'clock':'monotonic','executed':True,'started_receipt':start,'finished_receipt':finish})
            value['measurements'].append({'id':operation,'elapsed_ms':7,'evidence':[ref]})
        audit = {'authority':{'workspace':selection['workspace'],'transfer':selection},
                 'source_snapshot':value['source_snapshot']}
        return value, report, audit

    def test_selected_acceptance_rejects_fixture_stale_scope_missing_cases_and_waived_invariants(self):
        from p13_acceptance import validate_selected
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            value, report, audit = self.selected_acceptance(root)
            validate_selected(value, report, audit, root)
            for field, new in [('fixture',True),('input_sha256','stale'),('workflow',[]),('mapping',[]),
                               ('requirements',[]),('measurements',[]),('source_snapshot',None)]:
                changed = copy.deepcopy(value); changed[field] = new
                with self.assertRaises(GateError): validate_selected(changed, report, audit, root)
            changed = copy.deepcopy(value); changed['selection']['session'] = 'unrelated'
            with self.assertRaises(GateError): validate_selected(changed, report, audit, root)
            changed = copy.deepcopy(value)
            next(row for row in changed['requirements'] if row['id']=='L01')['disposition']='unsupported'
            with self.assertRaises(GateError): validate_selected(changed, report, audit, root)
            changed = copy.deepcopy(value)
            next(row for row in changed['requirements'] if row['id']=='A01')['disposition']='unsupported'
            with self.assertRaises(GateError): validate_selected(changed, report, audit, root)
            changed = copy.deepcopy(value); changed['safety']['external_service_stops'] = 1
            with self.assertRaises(GateError): validate_selected(changed, report, audit, root)
            changed = copy.deepcopy(value); changed['recovery']['new_destination'] = value['selection']['original_state']
            with self.assertRaises(GateError): validate_selected(changed, report, audit, root)

    def test_selected_measurements_reject_tampering_unknown_outcomes_and_fake_drain_timing(self):
        from p13_acceptance import validate_selected
        from common import sha256
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            value, report, audit = self.selected_acceptance(root)
            finished = root/'drain-finish.json'
            original = finished.read_text()
            finished.write_text('{}')
            with self.assertRaises(GateError): validate_selected(value, report, audit, root)
            finished.write_text(original)
            start = root/'drain-start.json'
            row = json.loads(start.read_text()); row['arguments']=['begin-transfer','/selected/workspace']
            start.write_text(json.dumps(row))
            # Even consistently rehashed reports cannot label begin-transfer as
            # the duration of an earlier host shutdown.
            ref = next(r for r in report['artifacts'] if r['path']==start.name)
            ref['sha256']=sha256(start)
            finish = json.loads(finished.read_text()); finish['started_sha256']=ref['sha256']
            finished.write_text(json.dumps(finish))
            finish_ref = next(r for r in report['artifacts'] if r['path']==finished.name)
            finish_ref['sha256']=sha256(finished)
            timing_path = root/'drain-timing.json'
            timing = json.loads(timing_path.read_text())
            timing['started_receipt']=ref; timing['finished_receipt']=finish_ref
            timing_path.write_text(json.dumps(timing))
            next(r for r in report['artifacts'] if r['path']==timing_path.name)['sha256']=sha256(timing_path)
            with self.assertRaisesRegex(GateError,'does not measure host draining'):
                validate_selected(value, report, audit, root)

    def test_explicitly_authorized_service_stops_require_bound_identity_and_exit_receipts(self):
        from p13_acceptance import validate_selected
        from common import sha256
        with tempfile.TemporaryDirectory() as folder:
            root=Path(folder);value,report,audit=self.selected_acceptance(root)
            def retain(name, item):
                path=root/(name+'.json');path.write_text(json.dumps(item));ref={'path':path.name,'sha256':sha256(path)}
                report['artifacts'].append(ref);return ref
            start=retain('service-start',{'pid':17,'exe':'/fixture/server','start_ticks':'42','authorization':'Synthetic explicit authorization'})
            finish=retain('service-finish',{'pid':17,'graceful_exit_observed':True,'forced':False})
            observation={k:value[k] for k in ['version','status','fixture','selection','input_sha256']}
            observation.update(case='authorized-service-stop',pid=17,user_authorized=True,identity_verified=True,exited=True,
                               started_receipt=start,finished_receipt=finish)
            ref=retain('service-observation',observation)
            value['safety'].update(external_service_stops=1,authorized_service_stops=[{'evidence':[ref]}])
            validate_selected(value,report,audit,root)
            for field in ['user_authorized','identity_verified','exited']:
                changed={**observation,field:False};bad=retain('service-bad-'+field,changed)
                value['safety']['authorized_service_stops']=[{'evidence':[bad]}]
                with self.assertRaises(GateError):validate_selected(value,report,audit,root)

    def test_authority_audit_rejects_fixture_stale_owner_and_unrelated_live_evidence(self):
        _,capture=self.live_configuration()
        value={'version':1,'status':'production_authority_verified','fixture':False,'original_state_unchanged':True,
            'authority':{'owner':'native','stage':'active','quiescent':True,'isolated':False,'workspace':'/fixture/workspace',
                'state':'/fixture/native','transfer':{'original_state':'/fixture/original','destination':'/fixture/native','session':'selected'}},
            'selected_session':'selected','source_snapshot':{'path':'/fixture/archive','sha256':'0'*64},
            'storage':{'integrity':'ok','staged_files':0},'runtime_acceptance':{'canary_report_sha256':'1'*64,
                'capture_sha256':capture['configuration_sha256'],'plan_sha256':capture['plan_sha256'],
                'precommit_recovery':'/fixture/recovery','recovery_manifest_sha256':'2'*64}}
        validate_authority_audit(value,'1'*64,capture)
        for change in [('fixture',True),('source_snapshot',None),('selected_session','different'),('original_state_unchanged',False)]:
            changed=copy.deepcopy(value);changed[change[0]]=change[1]
            with self.assertRaises(GateError):validate_authority_audit(changed,'1'*64,capture)
        for field,new in [('owner','type_script'),('stage','revoked'),('state','/fixture/other'),('quiescent',False),('isolated',True)]:
            changed=copy.deepcopy(value);changed['authority'][field]=new
            with self.assertRaises(GateError):validate_authority_audit(changed,'1'*64,capture)
        changed=copy.deepcopy(value);changed['storage']['staged_files']=1
        with self.assertRaises(GateError):validate_authority_audit(changed,'1'*64,capture)
        with self.assertRaises(GateError):validate_authority_audit(value,'3'*64,capture)
    def live_configuration(self):
        capture=read_json(ROOT/'implementation/evidence/P13/runtime-configuration-import.json')
        models=[{'id':m['model'],'provider':'llamacpp','api':'openai-completions',
                 'contextWindow':m['context_window'],'maxTokens':m['max_tokens']} for m in capture['models']]
        value={'configuration':{'sha256':capture['configuration_sha256'],'plan_sha256':capture['plan_sha256']},
               'models':models,'report':{'models':[m['id'] for m in models]},'runtime_events':[]}
        for model in models:
            for _ in range(2):
                for action in ['owned_start']+['owned_ready']*6+['owned_stop']:
                    value['runtime_events'].append({'model':model['id'],'action':action,'elapsed_ms':1,
                        'plan_sha256':capture['plan_sha256'][model['id']]})
        return value,capture
    def test_live_evidence_cannot_change_the_verified_catalog_or_configuration(self):
        value,capture=self.live_configuration()
        validate_live_configuration(value,capture)
        for field,new in [('contextWindow',32768),('maxTokens',1024),('api','different'),('provider','fixture')]:
            changed=copy.deepcopy(value);changed['models'][0][field]=new
            with self.assertRaises(GateError):validate_live_configuration(changed,capture)
        for field in ['sha256','plan_sha256']:
            changed=copy.deepcopy(value);changed['configuration'][field]='different'
            with self.assertRaises(GateError):validate_live_configuration(changed,capture)
    def test_live_evidence_requires_every_cold_launch_readiness_and_owned_stop(self):
        value,capture=self.live_configuration()
        validate_live_configuration(value,capture)
        for mutation in range(5):
            changed=copy.deepcopy(value)
            if mutation==0:changed['runtime_events'].pop()
            if mutation==1:changed['runtime_events'][0],changed['runtime_events'][1]=changed['runtime_events'][1],changed['runtime_events'][0]
            if mutation==2:changed['runtime_events'][0]['plan_sha256']='0'*64
            if mutation==3:changed['runtime_events'][0]['elapsed_ms']=-1
            if mutation==4:changed['runtime_events'][0]['elapsed_ms']=True
            with self.assertRaises(GateError):validate_live_configuration(changed,capture)
    def test_checkboxes_without_executed_canary_evidence_cannot_pass(self):
        with self.assertRaises(GateError): require_complete(self.report())
    def test_live_matrix_requires_both_adapters_in_order_and_authenticated_samples(self):
        models=list(read_json(ROOT/'planning/evidence/llama-runtime-lock.json')['models'])
        value={'version':1,'fixture':False,'status':'passed','failures':[], 'models':models,
               'runtime_settled':True,'cold_start_per_runner':True,'rows':[]}
        for model in models:
            for runner in ['typescript','rust']:
                for case in ['exact-copy','reasoning','tool-call','tool-result','context-limit','cancellation']:
                    value['rows'].append({'model':model,'runner':runner,'case':case,'answer':{
                        'settled':True,'payload':{'model':model},'runtime_before':{'owned_pid':7},
                        'runtime_after':None if case=='cancellation' else {'owned_pid':7},
                        'first_content_ms':1,'cancelled':case=='cancellation'}})
        validate_live_canaries(value,models)
        changed=copy.deepcopy(value);changed['rows'][1]=changed['rows'][0]
        with self.assertRaises(GateError):validate_live_canaries(changed,models)
        for edit in [{'runtime_before':None},{'settled':False},{'cancelled':True}]:
            changed=copy.deepcopy(value);changed['rows'][0]['answer'].update(edit)
            with self.assertRaises(GateError):validate_live_canaries(changed,models)
        changed=copy.deepcopy(value);changed['fixture']=True
        with self.assertRaises(GateError):validate_live_canaries(changed,models)
    def test_ts_runtime_guard_matches_locked_port_precedence_and_ordered_plans(self):
        from tool_gate import environment
        _, env = environment()
        result = subprocess.run(['node', str(ROOT/'tools/probes/p13_runtime_guard_test.mjs')],
                                env=env, check=True, capture_output=True, text=True, timeout=60)
        self.assertIn('Four TS guard/default/precedence/tier cases passed', result.stdout)
    def test_pinned_runtime_capture_uses_fake_actions_and_keeps_private_inputs(self):
        from tool_gate import environment
        _, env = environment()
        with tempfile.TemporaryDirectory() as folder:
            directory = Path(folder)
            executable = directory/'llama-server'
            executable.write_text('#!/bin/sh\ntouch '+str(directory/'unexpected-execution')+'\nexit 1\n')
            executable.chmod(0o700)
            models = {}
            profiles = {}
            for index in range(3):
                model = f'fixture-{index}'
                weight = directory/(model+'.gguf')
                weight.write_bytes(b'GGUF; fake material must never load')
                models[model] = str(weight)
                profiles[model] = {'serverPath':str(executable), 'contextWindow':32768,
                                   'maxTokens':4096, 'priority':False, 'slotSimilarityFlag':'none',
                                   'extraArgs':['--ordered-fixture','3','1','2']}
            profile = directory/'profiles.json'
            profile.write_text(json.dumps({'version':1,'models':profiles}))
            before = profile.read_bytes()
            request = directory/'request.json'
            request.write_text(json.dumps({'profile':str(profile), 'models':models,
                'environment':{'P13_PRIVATE_SENTINEL':'must-not-be-printed'}, 'vram_mib':16384}))
            output = directory/'capture.json'
            result = subprocess.run(['node', str(ROOT/'tools/probes/p13_runtime_capture.mjs'),
                str(ROOT/'reference/ag1-executable'), str(request), str(output)],
                env=env, check=True, capture_output=True, text=True, timeout=60)
            self.assertNotIn('must-not-be-printed', result.stdout+result.stderr)
            self.assertFalse((directory/'unexpected-execution').exists())
            self.assertEqual(profile.read_bytes(), before)
            self.assertEqual(os.stat(output).st_mode & 0o777, 0o600)
            capture = json.loads(output.read_text())
            self.assertEqual(capture['model_actions'], {'spawn':0,'stop':0,'network':0,'tune':0})
            self.assertEqual({row['model_id'] for row in capture['captures']}, set(models))
            for row in capture['captures']:
                self.assertEqual(row['args'][-4:], ['--ordered-fixture','3','1','2'])
                self.assertEqual(row['environment'], {'P13_PRIVATE_SENTINEL':'must-not-be-printed'})
                self.assertEqual(row['context_window'], 32768)
                self.assertEqual(row['max_tokens'], 4096)
    def log(self):
        return ''.join(f'test {name} ... ok\n' for name in sorted(CASES))+f'test result: ok. {len(CASES)} passed; 0 failed; 0 ignored; 0 measured; 0 filtered out;\n'
    def test_trace_keeps_all_requirements_and_does_not_promote_inherited_evidence(self):
        from cutover_trace import inventory
        trace = inventory(CASES)
        self.assertEqual(len(trace['rows']), 124)
        self.assertEqual(trace['status'], 'incomplete')
        self.assertTrue(all(row['selected_user_bundle_acceptance']=='pending' for row in trace['rows']))
        with self.assertRaises(GateError): inventory([])
    def test_exact_fixture_inventory(self):
        self.assertEqual(set(validate_contract(self.log())), CASES)
    def test_empty_and_filtered_cannot_pass(self):
        for log in ['', self.log().replace('0 filtered out','1 filtered out'), self.log().replace('0 ignored','1 ignored')]:
            with self.assertRaises(GateError): validate_contract(log)
    def test_duplicate_or_missing_cases_cannot_pass(self):
        names=sorted(CASES)
        for log in [self.log().replace(names[0],names[1]), self.log().replace(f'test {names[0]} ... ok\n','')]:
            with self.assertRaises(GateError): validate_contract(log)
    def report(self):
        return {'status':'passed','pending_gates':[], 'authority':{'accepted_rust_cutover':True}, 'per_model_canaries':[
            {'model':model,'status':'passed','ownership':'exclusive','fixture':False} for model in read_json(ROOT/'planning/evidence/llama-runtime-lock.json')['models']]}
    def test_fixture_success_cannot_waive_any_mandatory_gate(self):
        for update in [{'status':'blocked'},{'pending_gates':['live acceptance']},{'authority':{}},{'per_model_canaries':[]}]:
            report=self.report();report.update(update)
            with self.assertRaises(GateError): require_complete(report)
    def test_each_model_needs_live_exclusive_acceptance(self):
        for index in range(3):
            for change in [{'fixture':True},{'status':'pending'},{'ownership':'observed'}]:
                report=copy.deepcopy(self.report());report['per_model_canaries'][index].update(change)
                with self.assertRaises(GateError): require_complete(report)
    def test_duplicate_models_do_not_satisfy_coverage(self):
        report=self.report();report['per_model_canaries'][1]=report['per_model_canaries'][0].copy()
        with self.assertRaises(GateError): require_complete(report)

if __name__=='__main__': unittest.main()
