import copy,hashlib,sys,unittest
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from common import GateError
from evidence_gate import validate_measurements
class EvidenceGateTests(unittest.TestCase):
    def fixture(self):return {'status':'measured','model_actions':0,'network_actions':0,'scope':'fixture','state_entries':50,'state_bytes':10,'state_sha256':'a'*64,'serialization_ns':[1]*100,'files':[{'bytes':size,'sha256':hashlib.sha256(b'a'*size).hexdigest(),'snapshot_ns':[1]*5,'hash_ns':[1]*5,'repeated_validation_ns':[1]*5} for size in [1024,1048576,8388608]]}
    def test_valid(self):
        d=self.fixture();validate_measurements(d,copy.deepcopy(d))
    def test_missing_size(self):
        d=self.fixture();d['files'].pop()
        with self.assertRaises(GateError):validate_measurements(d,d)
    def test_wrong_hash(self):
        d=self.fixture();d['files'][0]['sha256']='wrong'
        with self.assertRaises(GateError):validate_measurements(d,d)
    def test_missing_timing(self):
        d=self.fixture();d['files'][0]['repeated_validation_ns'].pop()
        with self.assertRaises(GateError):validate_measurements(d,d)
    def test_state_mismatch(self):
        d=self.fixture();other=copy.deepcopy(d);other['state_sha256']='b'*64
        with self.assertRaises(GateError):validate_measurements(d,other)
    def test_empty_serialization(self):
        d=self.fixture();d['serialization_ns']=[]
        with self.assertRaises(GateError):validate_measurements(d,d)
    def test_effects(self):
        d=self.fixture();d['model_actions']=1
        with self.assertRaises(GateError):validate_measurements(d,d)
