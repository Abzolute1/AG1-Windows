import copy, sys, unittest
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from common import GateError
from context_gate import PROBES,validate_probe

class ContextGateTests(unittest.TestCase):
    def test_all_required_probe_counts(self):
        for name,counts in PROBES.items(): validate_probe(name,{'status':'passed',**counts,'model_actions':0,'network_actions':0})
    def test_empty_and_failed_probe_rejected(self):
        for data in [{},{'status':'failed'},{'status':'passed','compiler_cases':0,'model_actions':0,'network_actions':0}]:
            with self.assertRaises(GateError): validate_probe('compiler',data)
    def test_missing_case_and_real_calls_rejected(self):
        for name,counts in PROBES.items():
            expected={'status':'passed',**counts,'model_actions':0,'network_actions':0}
            for field in counts:
                bad=copy.deepcopy(expected);bad[field]-=1
                with self.assertRaises(GateError):validate_probe(name,bad)
            for field in ['model_actions','network_actions']:
                bad=copy.deepcopy(expected);bad[field]=1
                with self.assertRaises(GateError):validate_probe(name,bad)

class MeasurementGateTests(unittest.TestCase):
    def fixture(self,runtime='rust'):
        import json,hashlib
        from common import ROOT,read_json
        from context_gate import HISTORIES
        oracle={g['id']:g for g in read_json(ROOT/'fixtures/context/compiler-oracle.json')['groups']}
        groups=[]
        for id_ in HISTORIES:
            expected=oracle[id_];g={'id':id_,'input_sha256':expected['inputSha256'],'input_bytes':100,'input_messages':201,'artifact_bytes':200,'artifacts':1}
            for pass_ in ['cold','warm']:
                g[pass_+'_stats']=expected[pass_]['stats'];g[pass_+'_sha256']=hashlib.sha256(json.dumps(expected[pass_]['messages'],sort_keys=True,separators=(',',':'),ensure_ascii=False).encode()).hexdigest()
            for pass_ in ['cold','warm','rehydration']:g[pass_]=[{'ns':100} for _ in range(25)]
            if runtime=='rust':g['durable']={'original_sha256':g['input_sha256'],'original_bytes':100,'logical_file_bytes':1000,'physical_files':3,**{k:{'ns':100} for k in ['cold','warm','reopen','rehydrate_original']}}
            groups.append(g)
        return {'status':'measured','runtime':runtime,'scope':'synthetic validator unit test only','model_actions':0,'network_actions':0,'samples':25,'allocator_instrumentation':False,'groups':groups}
    def test_exact_workload_and_three_runs(self):
        from context_gate import compare_measurements
        a=self.fixture();b=self.fixture('typescript')
        self.assertTrue(compare_measurements([a]*3,[b]*3)['passed'])
        with self.assertRaises(GateError):compare_measurements([a],[b])
    def test_regression_is_not_hidden_by_noise_or_empty_samples(self):
        from context_gate import compare_measurements,validate_measurement
        a=self.fixture();b=self.fixture('typescript')
        for row in a['groups'][0]['cold']:row['ns']=111
        self.assertFalse(compare_measurements([a]*3,[b]*3)['passed'])
        a['groups'][0]['cold']=[]
        with self.assertRaises(GateError):validate_measurement(a,'rust')
    def test_corrupt_evidence_and_instrumented_timing_rejected(self):
        from context_gate import validate_measurement
        for field in ['input_sha256','cold_sha256','warm_sha256']:
            a=self.fixture();a['groups'][0][field]='0'*64
            with self.assertRaises(GateError):validate_measurement(a,'rust')
        a=self.fixture();a['allocator_instrumentation']=True
        with self.assertRaises(GateError):validate_measurement(a,'rust')
    def test_durability_receipt_and_real_calls_required(self):
        from context_gate import validate_measurement
        for field in ['original_sha256','original_bytes','rehydrate_original']:
            a=self.fixture();a['groups'][0]['durable'].pop(field)
            with self.assertRaises(GateError):validate_measurement(a,'rust')
        for field in ['network_actions','model_actions']:
            a=self.fixture();a[field]=1
            with self.assertRaises(GateError):validate_measurement(a,'rust')
