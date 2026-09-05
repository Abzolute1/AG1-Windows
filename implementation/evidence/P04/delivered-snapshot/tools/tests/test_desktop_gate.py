import sys,unittest
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from desktop_gate import worker_result,measurement,GateError
class DesktopGateTests(unittest.TestCase):
 def test_empty_worker_selection_rejected(self):
  with self.assertRaises(GateError):worker_result('')
 def test_skipped_worker_tests_rejected(self):
  with self.assertRaises(GateError):worker_result('ℹ tests 19\nℹ pass 18\nℹ fail 0\nℹ cancelled 0\nℹ skipped 1\nℹ todo 0\n')
 def test_real_worker_summary(self):
  self.assertEqual(worker_result('ℹ tests 19\nℹ pass 19\nℹ fail 0\nℹ cancelled 0\nℹ skipped 0\nℹ todo 0\n'),19)
 def test_short_idle_rejected(self):
  with self.assertRaises(GateError):measurement({'duration_seconds':299,'samples':[]})
 def test_missing_process_tree_rejected(self):
  with self.assertRaises(GateError):measurement({'duration_seconds':301,'samples':[{'time':0,'processes':[]},{'time':301,'processes':[]}]})
 def test_forged_duration_rejected(self):
  with self.assertRaises(GateError):measurement({'duration_seconds':301,'samples':[{'time':0,'processes':[1],'pss_kib':1},{'time':1,'processes':[1],'pss_kib':1}]})
