import json
from pathlib import Path
import sys
import unittest
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from common import GateError
from platform_gate import validate_platform_log

class PlatformGateTests(unittest.TestCase):
    def sample(self):
        log='\n'.join(f'test case_{chr(97+i)} ... ok' for i in range(26))
        log+='\ntest result: ok. 26 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out;'
        for count in [120,900]:log+='\nP03_MEASUREMENT '+json.dumps({'files':count,'samples_ns':{name:[1,2,3,4,5] for name in ['status_diff','index','search','read','write']}})
        return log+'\nP03_LOG '+json.dumps({'memory_bytes':65536,'disk_bytes':8388608,'seen':10485768})
    def test_rejects_missing_measurements_and_overflow(self):
        for log in ['',self.sample().replace('P03_LOG','absent'),self.sample().replace('65536','65537'),self.sample().replace('900','120')]:
            with self.assertRaises(GateError):validate_platform_log(log)
    def test_accepts_complete_bounded_experiment(self):
        self.assertEqual(validate_platform_log(self.sample())[0],26)
