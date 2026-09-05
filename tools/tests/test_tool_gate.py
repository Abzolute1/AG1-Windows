import sys
from pathlib import Path
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from common import GateError
from tool_gate import node_cases, probe_result, rust_cases


class ToolGateTests(unittest.TestCase):
    def test_rust_requires_exact_inventory_and_unfiltered_success(self):
        log = 'test example ... ok\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out;\n'
        self.assertEqual(rust_cases('test.rs', log, 1), {('test.rs', 'example')})
        for value in [log.replace('0 ignored', '1 ignored'), log + log, log.replace('... ok', '... FAILED')]:
            with self.assertRaises(GateError): rust_cases('test.rs', value, 1)
        with self.assertRaises(GateError): rust_cases('test.rs', log, 2)

    def test_node_rejects_skip_duplicate_and_missing_case_evidence(self):
        log = 'ok 1 - actual test\n# tests 1\n# pass 1\n# fail 0\n# cancelled 0\n# skipped 0\n# todo 0\n'
        self.assertEqual(node_cases('test.mjs', log), {('test.mjs', 'actual test')})
        for value in [log.replace('# skipped 0', '# skipped 1'), log + log,
                      log.replace('ok 1 - actual test\n', ''), log.replace('# cancelled 0', '# cancelled 1')]:
            with self.assertRaises(GateError): node_cases('test.mjs', value)

    def test_probe_requires_one_nonempty_completion_without_external_actions(self):
        value = '{"status":"passed","cases":3,"network_actions":0}'
        self.assertEqual(probe_result('fixture warning\n' + value)['cases'], 3)
        for log in [value + '\n' + value, value.replace(':3', ':0'),
                    value.replace('actions":0', 'actions":1'), '{"status":"failed"}']:
            with self.assertRaises(GateError): probe_result(log)


if __name__ == '__main__': unittest.main()
