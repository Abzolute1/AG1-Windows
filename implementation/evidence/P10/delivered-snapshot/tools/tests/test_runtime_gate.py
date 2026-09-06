"""Behavioral rejection tests for the P10 completion gate."""
import copy
from pathlib import Path
import sys
import unittest
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from common import ROOT, GateError, read_json
from runtime_gate import validate_trace_map, package_cases, validate_replay
from runtime_measurement import validate, METRICS
import hashlib

class RuntimeGateTests(unittest.TestCase):
    def test_every_requirement_needs_executed_success_and_failure(self):
        value=read_json(ROOT/'fixtures/runtime/trace-map.json')
        passed={(r['path'],r['test']) for row in value['rows'] for side in ['success','failure'] for r in row[side]}
        validate_trace_map(value,passed)
        for change in ['missing','duplicate','no_failure','unexecuted']:
            bad=copy.deepcopy(value)
            if change=='missing': bad['rows'].pop()
            elif change=='duplicate': bad['rows'][-1]=bad['rows'][0]
            elif change=='no_failure': bad['rows'][0]['failure']=[]
            else: bad['rows'][0]['failure'][0]['test']='not_an_executed_case'
            with self.subTest(change=change),self.assertRaises(GateError):validate_trace_map(bad,passed)

    def test_cargo_suite_accounting_rejects_missing_skipped_duplicate_and_empty(self):
        log='    Running tests/runtime_contract.rs (target/test)\n\nrunning 1 test\ntest proof ... ok\n\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1s\n'
        self.assertEqual(len(package_cases('ag1-core',log,['runtime_contract'])),1)
        for bad in ['',log+log,log.replace('0 ignored','1 ignored'),log.replace('1 passed','0 passed'),log.replace('proof ... ok','proof ... ignored')]:
            with self.subTest(log=bad),self.assertRaises(GateError):package_cases('ag1-core',bad,['runtime_contract'])

    def test_differential_cannot_be_reused_after_any_input_changes(self):
        from reference_oracles import input_fingerprints
        before={'example':'digest'}
        report={'status':'passed','phase_complete':False,'input_fingerprints':{'product':before,'reference':input_fingerprints()},'commands':[{'exit_code':0}]}
        validate_replay(report,before)
        for change in ['stale','failed','empty','phase_complete']:
            bad=copy.deepcopy(report)
            if change=='stale': bad['input_fingerprints']['product']['example']='changed'
            elif change=='failed': bad['commands'][0]['exit_code']=1
            elif change=='empty': bad['commands']=[]
            else: bad['phase_complete']=True
            with self.subTest(change=change),self.assertRaises(GateError):validate_replay(bad,before)

    def test_measurement_rejects_lost_evidence_unmatched_inputs_and_unbounded_growth(self):
        good={'status':'measured','runtime':'rust','scope':'test','model_actions':0,'external_network_actions':0,
            'input_sha256':hashlib.sha256(('Owned evidence β\n'*256).encode()).hexdigest(),
            'samples':[{**dict.fromkeys(METRICS,100),'requests':3,'tool_results':1,'full_read_evidence':True,'users':['Read input.txt','Continue']} for _ in range(25)],
            'cycles':[{'cycle':i,'ns':100,'heap_bytes':1024,'journal_entries':6+4*i,'projected_messages':2} for i in range(32)]}
        validate(good,'rust')
        for change in ['input','samples','history','projection','heap','growth','latency','tool','evidence','model']:
            bad=copy.deepcopy(good)
            if change=='input': bad['input_sha256']='wrong'
            elif change=='samples': bad['samples'].pop()
            elif change=='history': bad['cycles'][-1]['journal_entries']=6
            elif change=='projection': bad['cycles'][-1]['projected_messages']=33
            elif change=='heap': bad['cycles'][-1]['heap_bytes']=129*1024*1024
            elif change=='growth': bad['cycles'][-1]['heap_bytes']=17*1024*1024
            elif change=='latency': bad['samples'][0]['total_ns']=0
            elif change=='tool': bad['samples'][0]['tool_results']=0
            elif change=='evidence': bad['samples'][0]['full_read_evidence']=False
            else: bad['model_actions']=1
            with self.subTest(change=change),self.assertRaises(GateError):validate(bad,'rust')

if __name__=='__main__':unittest.main()
