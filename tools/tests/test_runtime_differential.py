"""The runtime replay must reject missing, duplicated and misleading evidence."""
import copy
import json
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from common import ROOT, GateError, read_json
from runtime_differential import validate_inventory, validate_probe


class RuntimeDifferentialTests(unittest.TestCase):
    def test_missing_or_duplicate_runtime_coverage_and_false_phase_completion_are_rejected(self):
        inventory = read_json(ROOT / 'fixtures/runtime/differential-inventory.json')
        validate_inventory(inventory)
        for change in ['missing', 'duplicate', 'empty_contract', 'missing_contract', 'phase_complete']:
            bad = copy.deepcopy(inventory)
            if change == 'missing': bad['probes'].pop()
            elif change == 'duplicate': bad['probes'][-1] = bad['probes'][0]
            elif change == 'empty_contract': bad['contracts'][0]['count'] = 0
            elif change == 'missing_contract': bad['contracts'].pop()
            else: bad['phase_complete'] = True
            with self.subTest(change=change), self.assertRaises(GateError):
                validate_inventory(bad)

    def test_empty_changed_ambiguous_and_effectful_probe_summaries_cannot_pass(self):
        probe = {'path': 'fixture', 'expected': {'status': 'passed', 'cases': 3, 'model_actions': 0, 'network_actions': 0}}
        good = json.dumps(probe['expected'])
        self.assertEqual(validate_probe(probe, good), probe['expected'])
        for log in ['', good + '\n' + good, good.replace('"cases": 3', '"cases": 2'),
                    good.replace('"model_actions": 0', '"model_actions": 1')]:
            with self.subTest(log=log), self.assertRaises(GateError):
                validate_probe(probe, log)


if __name__ == '__main__': unittest.main()
