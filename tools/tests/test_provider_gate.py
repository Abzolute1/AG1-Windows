import copy, hashlib, sys, unittest
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from provider_gate import validate_rust_log, validate_groups, validate_measurements, FAMILIES
from common import GateError, PIN
class ProviderGateTests(unittest.TestCase):
    def log(self):return 'test fixture::one ... ok\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out;'
    def test_valid_contract(self):self.assertEqual(validate_rust_log(self.log(),1),1)
    def test_missing_contract(self):
        with self.assertRaises(GateError):validate_rust_log('',1)
    def test_skipped_contract(self):
        with self.assertRaises(GateError):validate_rust_log(self.log().replace('0 ignored','1 ignored'),1)
    def test_duplicate_contract(self):
        with self.assertRaises(GateError):validate_rust_log('test fixture::one ... ok\n'+self.log(),1)
    def groups(self):return {'source_commit':PIN,'network_actions':0,'groups':[{'api':a} for a in FAMILIES]}
    def test_valid_groups(self):validate_groups(self.groups())
    def test_missing_family(self):
        d=self.groups();d['groups'].pop()
        with self.assertRaises(GateError):validate_groups(d)
    def test_stale_pin(self):
        d=self.groups();d['source_commit']='stale'
        with self.assertRaises(GateError):validate_groups(d)
    def measurement(self):
        w={'fragment':'🌙','fragments':10000}
        d={'status':'measured','model_actions':0,'network_actions':0,'workload':w,'output_bytes':40000,'output_sha256':hashlib.sha256(('🌙'*10000).encode()).hexdigest(),'peak_waiters':128,'max_rss_kib':1,'scope':'fixture','normalization_ns':[1]*5,'queue_batch_ns':[1]*100,'wire_bytes':50000}
        return d,w
    def test_valid_measurement(self):
        d,w=self.measurement();validate_measurements(d,copy.deepcopy(d),w)
    def test_wrong_output(self):
        d,w=self.measurement();bad=copy.deepcopy(d);bad['output_sha256']='wrong'
        with self.assertRaises(GateError):validate_measurements(d,bad,w)
    def test_missing_samples(self):
        d,w=self.measurement();d['queue_batch_ns'].pop()
        with self.assertRaises(GateError):validate_measurements(d,d,w)
    def test_effectful_measurement(self):
        d,w=self.measurement();d['model_actions']=1
        with self.assertRaises(GateError):validate_measurements(d,d,w)
