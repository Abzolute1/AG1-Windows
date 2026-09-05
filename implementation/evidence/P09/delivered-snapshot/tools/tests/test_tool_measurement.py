import copy
import hashlib
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from common import GateError
from tool_measurement import validate


class ToolMeasurementTests(unittest.TestCase):
    def fixture(self):
        text = ('abcdefghijklmnopqrstuvwxyz0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ!\n' * 512).encode()
        data = dict(status='measured', runtime='rust', samples=25, scope='owned fixture',
                    model_actions=0, external_network_actions=0, no_effect_before_approval=True,
                    input_bytes=len(text), input_sha256=hashlib.sha256(text).hexdigest(), read_result_sha256='a'*64)
        for key in ['noop_prepare', 'durable_noop', 'sequential_reads', 'parallel_reads', 'serialized_mutations']:
            data[key] = dict(ns=[1]*25, calls_per_sample=1000 if key=='noop_prepare' else 8)
        data['parallel_reads']['peak_held_proposals'] = 8
        data['serialized_mutations'].update(peak_held_proposals=1, final_bytes=399)
        data['large_spool'] = dict(ns=[1]*5, bytes_per_sample=4194304, spools_cleaned=True,
                                  sha256=hashlib.sha256(b'x'*4194304).hexdigest())
        data['extension'] = dict(roundtrip_ns=[1]*25, startup_ns=1, stopped=True)
        return data

    def test_measurement_requires_complete_custody_concurrency_and_lifecycle(self):
        original = self.fixture()
        validate(original)
        for group, key, value in [('large_spool', 'sha256', 'b'*64), ('large_spool', 'spools_cleaned', False),
                                  ('extension', 'stopped', False), ('serialized_mutations', 'peak_held_proposals', 2),
                                  ('parallel_reads', 'ns', [1]*24), ('noop_prepare', 'calls_per_sample', 1)]:
            data = copy.deepcopy(original)
            data[group][key] = value
            with self.assertRaises(GateError): validate(data)

    def test_measurement_rejects_missing_admission_or_changed_inputs(self):
        for key, value in [('no_effect_before_approval', False), ('input_sha256', 'b'*64),
                           ('external_network_actions', 1), ('model_actions', 1)]:
            data = self.fixture(); data[key] = value
            with self.assertRaises(GateError): validate(data)


if __name__ == '__main__': unittest.main()
