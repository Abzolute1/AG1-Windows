import copy
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import sys
import tarfile
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from common import ROOT, GateError
from release_gate import SCENARIOS, artifact, rust_results, validate_duration, validate_performance, validate_report

spec = importlib.util.spec_from_file_location('release_install', ROOT/'product/release/install.py')
installer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(installer)


def quality():
    return {**{k:{'passed':2,'total':2} for k in ('required_recall','forbidden_avoidance','contradiction_handling','task_success')},
            **{k:0 for k in ('stale_authoritative_claims','leaked_owned_children','lost_commands','replayed_effects','runtime_preservation_violations')}}


def duration(hours=8):
    return {'start_monotonic_ns':1,'end_monotonic_ns':1+hours*3600*10**9,
            'activity_policy':{'max_idle_seconds':120,'max_activity_seconds':1800,'minimum_tasks_per_hour':2},
            'activities':[{'id':f'activity-{i}','kind':'task' if i%2==0 else sorted(SCENARIOS)[(i//2)%8],
                'started_monotonic_ns':1+i*900*10**9,'finished_monotonic_ns':1+(i+1)*900*10**9,
                'passed':True,'finished_receipt':{'path':f'activity-{i}.json','sha256':'a'*64}} for i in range(hours*4)],
            'externalized_tokens':2_500_000,'token_accounting':'externalized-content-once',
            'sessions':[{'scenario':s,'executed':True,'passed':True,'evidence':[{'path':'raw','sha256':'a'*64}]} for s in SCENARIOS],
            'samples':[{'monotonic_ns':1+i*60*10**9,'heap_bytes':1000,'pss_bytes':2000,'persistent_evidence_bytes':i*100,'owned_children':0,'host_present':True,'heap_observation':{'pid':10},'processes':[{'pid':10,'start_ticks':1,'executable_device':1,'executable_inode':1}]} for i in range(hours*60+1)],
            'warmup_seconds':60,'heap_growth_budget_bytes_per_hour':0,'quality':quality()}


class Reports(unittest.TestCase):
    def test_elapsed_clock_and_heartbeats_cannot_replace_continuous_work(self):
        for mutate in (lambda d:d.update(activities=[]),
                       lambda d:d.update(activities=d['activities'][:2]),
                       lambda d:d.update(activities=d['activities'][1:]),
                       lambda d:d['activities'][3].update(started_monotonic_ns=d['activities'][3]['started_monotonic_ns']+121*10**9),
                       lambda d:d['activities'][0].update(finished_monotonic_ns=d['end_monotonic_ns']),
                       lambda d:d['activities'][2].update(id=d['activities'][0]['id']),
                       lambda d:d['activity_policy'].update(max_idle_seconds=3600)):
            data=duration();mutate(data)
            with self.assertRaises(GateError): validate_duration(data,8)

    def test_short_run_cannot_pass_either_duration(self):
        validate_duration(duration(),8)
        for h in (8,24):
            data=duration()
            data['end_monotonic_ns']=60*10**9
            with self.assertRaises(GateError): validate_duration(data,h)
        with self.assertRaises(GateError): validate_duration(duration(),24)

    def test_missing_samples_drift_nan_and_quality_failure_rejected(self):
        for mutate in (lambda d:d.update(samples=[]),
                       lambda d:d['samples'][4].update(monotonic_ns=d['samples'][3]['monotonic_ns']),
                       lambda d:d['samples'][4].update(heap_bytes=float('nan')),
                       lambda d:d['quality'].update(stale_authoritative_claims=1),
                       lambda d:d['quality']['task_success'].update(passed=1),
                       lambda d:d.update(externalized_tokens=1_999_999)):
            data=duration();mutate(data)
            with self.assertRaises(GateError): validate_duration(data,8)

    def test_persistent_evidence_growth_is_separate_from_heap_leak(self):
        data=duration()
        self.assertEqual(validate_duration(data,8)['heap_slope_bytes_per_hour'],0)
        for i,s in enumerate(data['samples']): s['heap_bytes']+=i*100
        with self.assertRaises(GateError): validate_duration(data,8)

    def test_restarts_cannot_hide_excess_native_lifetime_growth(self):
        data=duration()
        data['heap_growth_budget_bytes_per_hour']=5000
        for i,sample in enumerate(data['samples']):
            sample['heap_bytes']=1000+(i%30)*100
            pid=10+i//30
            sample['heap_observation']['pid']=pid
            sample['processes'][0]['pid']=pid
            sample['processes'][0]['start_ticks']=pid
        with self.assertRaises(GateError):validate_duration(data,8)

    def test_empty_skipped_and_filtered_native_tests_rejected(self):
        good='test preservation ... ok\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out;'
        self.assertEqual(rust_results(good)['passed'],1)
        for text in ('',good.replace('0 ignored','1 ignored'),good.replace('0 filtered','1 filtered'),good.replace('1 passed','2 passed')):
            with self.assertRaises(GateError): rust_results(text)

    def test_report_cannot_pass_by_status_or_old_package_alone(self):
        for report in ({'status':'passed'}, {'version':1,'kind':'duration-8h','status':'passed','input_sha256':'i','package_sha256':'old'}):
            with self.assertRaises(GateError): validate_report('duration-8h',report,'i','p')

    def test_evidence_tamper_missing_escape_and_symlink_rejected(self):
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp);path=root/'raw';path.write_bytes(b'evidence')
            row={'path':'raw','sha256':hashlib.sha256(b'evidence').hexdigest()}
            self.assertEqual(artifact(row,root),path)
            path.write_bytes(b'changed')
            with self.assertRaises(GateError): artifact(row,root)
            path.unlink();path.symlink_to('/etc/hosts')
            with self.assertRaises(GateError): artifact(row,root)
            with self.assertRaises(GateError): artifact({'path':'../raw','sha256':'a'*64},root)

    def test_missing_packaged_performance_does_not_inherit_renderer_metric(self):
        with self.assertRaises(GateError): validate_performance({'shell_ready_ms':[6]*20})


class Installation(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root=Path(self.temp.name)

    def candidate(self,name='candidate',payload=b'candidate',extra=None):
        path=self.root/(name+'.tar.gz')
        rows={'bin/ag1-cli':payload}
        manifest={'version':1,'platform':'linux-x86_64','files':[{'path':n,'sha256':hashlib.sha256(v).hexdigest(),'bytes':len(v),'mode':0o755} for n,v in rows.items()]}
        rows['package-manifest.json']=json.dumps(manifest).encode()
        with tarfile.open(path,'w:gz') as tar:
            for n,v in rows.items():
                info=tarfile.TarInfo(n);info.size=len(v);info.mode=0o755
                tar.addfile(info,io.BytesIO(v))
            if extra: tar.addfile(extra)
        return path,installer.digest(path)

    def test_update_keeps_old_payload_receipt_and_external_runtime(self):
        a,ah=self.candidate();b,bh=self.candidate('new',b'new')
        external=self.root/'llama-server';external.write_bytes(b'tuned')
        dest=self.root/'installed'
        old=installer.install(a,ah,dest);new=installer.install(b,bh,dest)
        self.assertEqual((old/'bin/ag1-cli').read_bytes(),b'candidate')
        self.assertEqual((dest/'current').resolve(),new)
        self.assertEqual(external.read_bytes(),b'tuned')
        receipts=list((dest/'receipts').glob('*.json'))
        self.assertEqual(len(receipts),2)
        self.assertTrue(all(json.loads(p.read_text())['archive_sha256'] in (ah,bh) for p in receipts))

    def test_interrupt_at_every_boundary_retains_evidence_and_refuses_unknown_retry(self):
        a,ah=self.candidate();b,bh=self.candidate('new',b'new')
        for boundary in ('pending','verified','published','activated'):
            dest=self.root/boundary
            old=installer.install(a,ah,dest)
            def crash(stage):
                if stage==boundary: raise RuntimeError('injected interruption')
            with self.assertRaises(RuntimeError): installer.install(b,bh,dest,crash)
            self.assertTrue((dest/'pending.json').is_file())
            self.assertEqual((old/'bin/ag1-cli').read_bytes(),b'candidate')
            if boundary!='activated': self.assertEqual((dest/'current').resolve(),old)
            with self.assertRaisesRegex(ValueError,'Unknown pending'): installer.install(b,bh,dest)

    def test_bad_hash_cannot_create_destination(self):
        a,_=self.candidate();dest=self.root/'installed'
        with self.assertRaises(ValueError): installer.install(a,'0'*64,dest)
        self.assertFalse(dest.exists())

    def test_archive_replacement_after_preliminary_hash_cannot_activate(self):
        a,ah=self.candidate();b,_=self.candidate('replacement',b'unreviewed')
        dest=self.root/'installed'
        def swap(stage):
            if stage=='pending': b.replace(a)
        with self.assertRaisesRegex(ValueError,'identity changed'):
            installer.install(a,ah,dest,swap)
        self.assertFalse((dest/'current').exists())
        self.assertTrue((dest/'pending.json').is_file())

    def test_archive_traversal_and_links_cannot_escape_stage(self):
        for name,kind in (('../outside',tarfile.REGTYPE),('bin/link',tarfile.SYMTYPE),('/absolute',tarfile.REGTYPE)):
            extra=tarfile.TarInfo(name);extra.type=kind;extra.linkname='/etc/hosts'
            a,ah=self.candidate(str(len(list(self.root.iterdir()))),extra=extra)
            with self.assertRaises(ValueError): installer.install(a,ah,self.root/(a.stem+'install'))
        self.assertFalse((self.root/'outside').exists())

    def test_symlinked_destination_existing_data_and_protected_runtime_rejected(self):
        a,ah=self.candidate();external=self.root/'external';external.mkdir()
        link=self.root/'link';link.symlink_to(external,target_is_directory=True)
        for path in (link,external,Path('/home/alex/Projects/llama-cpp-turboquant/test')):
            with self.assertRaises(ValueError): installer.install(a,ah,path)
        self.assertEqual(list(external.iterdir()),[])


if __name__ == '__main__': unittest.main()
