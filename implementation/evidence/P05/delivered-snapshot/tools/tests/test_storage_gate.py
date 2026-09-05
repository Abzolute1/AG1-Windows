import sys,unittest
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from storage_gate import validate_measurements,validate_oracles,validate_rust_log,GateError

class StorageGateTests(unittest.TestCase):
 def test_empty_test_selection_fails(self):
  with self.assertRaises(GateError):validate_rust_log('test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out;',minimum=28)
 def test_skipped_test_fails(self):
  with self.assertRaises(GateError):validate_rust_log('test result: ok. 28 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out;',minimum=28)
 def test_missing_large_measurement_fails(self):
  with self.assertRaises(GateError):validate_measurements({'status':'measured','model_actions':0,'journal':[{'entries':10000}]})
 def test_model_action_is_not_a_storage_benchmark(self):
  with self.assertRaises(GateError):validate_measurements({'status':'measured','model_actions':1})
 def test_incomplete_export_oracle_fails(self):
  with self.assertRaises(GateError):validate_oracles({'status':'passed','source_files':38,'journals':4,'utf8_windows':55,'cursor_versions':2,'model_actions':0},{'status':'passed','model_actions':0})
 def test_complete_oracles_are_accepted(self):
  validate_oracles({'status':'passed','source_files':38,'journals':4,'utf8_windows':55,'cursor_versions':2,'model_actions':0},{'status':'passed','model_actions':0,'original_sdk_resume':True,'artifact_bytes':True,'cursor_compatible':True,'lease_compatible':True})
