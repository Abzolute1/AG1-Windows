import sys
from pathlib import Path
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from common import GateError
from protocol_gate import validate_rust_log


class ProtocolGateTests(unittest.TestCase):
    def test_rejects_empty_skipped_or_forged_summary(self):
        for log in ['', 'test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out;',
                    'test result: ok. 18 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out;',
                    'test result: ok. 18 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out;']:
            with self.assertRaises(GateError):
                validate_rust_log(log)

    def test_accepts_actual_names_and_rejects_duplicates(self):
        log = '\n'.join(f'test case_{chr(97+i)} ... ok' for i in range(18))
        log += '\ntest result: ok. 18 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out;'
        self.assertEqual(validate_rust_log(log), 18)
        with self.assertRaises(GateError):
            validate_rust_log(log.replace('case_b', 'case_a'))
