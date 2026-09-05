import copy
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from common import ROOT, GateError, run, sha256
from runtime_manifest import stable_hash
from phase_gate import verify_artifacts, verify_runtime_material
from reference_oracles import validate_vitest, validate_catalog_report, validate_defects


def report(status='passed'):
    return {'success':status=='passed','numTotalTests':1,'numPassedTests':int(status=='passed'),
            'numFailedTests':int(status=='failed'),'numPendingTests':int(status=='skipped'),
            'testResults':[{'name':'placeholder','assertionResults':[{'fullName':'case','title':'case','status':status,'failureMessages':['known error']}]}]}


class GateTests(unittest.TestCase):
    def setUp(self):
        (ROOT/'.local').mkdir(exist_ok=True)
        self.temp=tempfile.TemporaryDirectory(dir=ROOT/'.local')
        self.root=Path(self.temp.name)

    def tearDown(self):self.temp.cleanup()

    def test_empty_selection_is_not_success(self):
        with self.assertRaises(GateError):validate_vitest({'success':True,'testResults':[]})

    def test_missing_fixture_is_rejected(self):
        with self.assertRaises(GateError):verify_artifacts(self.root,[{'path':'missing','sha256':'0'*64}])

    def test_stale_fixture_hash_is_rejected(self):
        p=self.root/'fixture';p.write_text('old');digest=sha256(p);p.write_text('new')
        with self.assertRaises(GateError):verify_artifacts(self.root,[{'path':'fixture','sha256':digest}])

    def test_artifact_traversal_and_duplicates_are_rejected(self):
        p=self.root/'fixture';p.write_text('safe');row={'path':'fixture','sha256':sha256(p)}
        with self.assertRaises(GateError):verify_artifacts(self.root,[row,row])
        with self.assertRaises(GateError):verify_artifacts(self.root,[{'path':'../escape','sha256':'0'*64}])

    def test_failed_subprocess_never_passes(self):
        with self.assertRaises(GateError):run([sys.executable,'-c','raise SystemExit(7)'],log=self.root/'command.log')

    def test_subprocess_timeout_never_passes(self):
        with self.assertRaises(GateError):run([sys.executable,'-c','import time;time.sleep(10)'],log=self.root/'timeout.log',timeout=0.05)

    def test_valid_executed_report_passes(self):
        self.assertEqual(len(validate_vitest(report(),expected=1)),1)

    def test_skipped_and_failed_tests_do_not_pass(self):
        for status in ['skipped','failed']:
            with self.subTest(status=status),self.assertRaises(GateError):validate_vitest(report(status))

    def test_summary_cannot_forge_execution_count(self):
        data=report();data['numTotalTests']=2
        with self.assertRaises(GateError):validate_vitest(data)

    def test_unknown_catalog_failure_cannot_be_waived(self):
        path=self.root/'case.ts';path.write_text('fixture');data=report('failed');data['testResults'][0]['name']=str(path)
        with self.assertRaises(GateError):validate_catalog_report(data,[],reference=self.root)

    def test_named_exception_requires_hash_and_exact_reason(self):
        path=self.root/'case.ts';path.write_text('fixture');data=report('failed');data['testResults'][0]['name']=str(path)
        # At least one actual pass is required even alongside reviewed exceptions.
        data['testResults'][0]['assertionResults'].append({'fullName':'real pass','title':'real pass','status':'passed'})
        data['numTotalTests']=2;data['numPassedTests']=1
        exception={'source':'case.ts','source_sha256':sha256(path),'name':'case','expected_status':'failed','required_error':'known error'}
        checked=validate_catalog_report(data,[exception],reference=self.root)
        self.assertEqual(checked['passed'],1);self.assertEqual(len(checked['accepted_nonpassing']),1)
        wrong={**exception,'required_error':'different'}
        with self.assertRaises(GateError):validate_catalog_report(data,[wrong],reference=self.root)
        path.write_text('changed')
        with self.assertRaises(GateError):validate_catalog_report(data,[exception],reference=self.root)

    def test_runtime_hash_cache_invalidates_same_size_edit(self):
        p=self.root/'model';p.write_bytes(b'old');cache={};first,_=stable_hash(p,cache)
        p.write_bytes(b'new');second,reused=stable_hash(p,cache)
        self.assertNotEqual(first,second);self.assertFalse(reused)

    def test_runtime_gate_rejects_changed_weight_after_capture(self):
        p=self.root/'model.gguf';p.write_bytes(b'old');cache={};digest,_=stable_hash(p,cache)
        runtime={'weights':[{'path':str(p),'sha256':digest}],'loader_objects':{}}
        verify_runtime_material(runtime,cache)
        p.write_bytes(b'new')
        with self.assertRaises(GateError):verify_runtime_material(runtime,cache)

    def test_defect_missing_probe_is_not_expected_failure(self):
        with self.assertRaises(GateError):validate_defects({}, {})

    def test_offline_guard_preserves_promisified_exec(self):
        script="import {execFile} from 'node:child_process';import {promisify} from 'node:util'; const r=await promisify(execFile)(process.execPath,['-e','process.stdout.write(\"fixture\")']);if(r.stdout!=='fixture')process.exit(9);"
        result=subprocess.run(['node','--import',str(ROOT/'tools/offline_guard.mjs'),'--input-type=module','-e',script],capture_output=True,text=True)
        self.assertEqual(result.returncode,0,result.stderr)

    def test_offline_guard_denies_external_fetch_and_model_spawn(self):
        script="import {spawn} from 'node:child_process';let n=0;try{await fetch('http://127.0.0.1:18080/health')}catch{n++}try{spawn('/fixture/llama-server',[])}catch{n++}if(n!==2)process.exit(8);"
        result=subprocess.run(['node','--import',str(ROOT/'tools/offline_guard.mjs'),'--input-type=module','-e',script],capture_output=True,text=True)
        self.assertEqual(result.returncode,0,result.stderr)

    def test_offline_guard_allows_owned_fixture_server(self):
        script="import {createServer} from 'node:http';const s=createServer((q,r)=>r.end('fixture'));await new Promise(r=>s.listen(0,'127.0.0.1',r));try{const r=await fetch('http://127.0.0.1:'+s.address().port);if(await r.text()!=='fixture')process.exitCode=9;}finally{s.closeAllConnections();s.close();}"
        result=subprocess.run(['node','--import',str(ROOT/'tools/offline_guard.mjs'),'--input-type=module','-e',script],capture_output=True,text=True,timeout=10)
        self.assertEqual(result.returncode,0,result.stderr)


if __name__=='__main__':unittest.main()
