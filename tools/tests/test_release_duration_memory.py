import unittest
import sys
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from release_duration.memory_analysis import analyze

class MemoryAnalysisTests(unittest.TestCase):
    def sample(self, minute, pid, value):
        root={'pid':pid,'start_ticks':pid,'executable_device':1,'executable_inode':1}
        return {'monotonic_ns':int((minute*60+1)*1e9),'heap_bytes':value,'pss_bytes':value+1000,
                'persistent_evidence_bytes':minute*1000,'host_present':pid is not None,
                'processes':[root] if pid else [],'heap_observation':{'pid':pid} if pid else None}
    def test_restart_does_not_hide_a_leak(self):
        rows=[self.sample(i,1 if i<60 else 2,1000000+(i%60)*100000) for i in range(120)]
        result=analyze(rows,1e9,60,5000000)
        self.assertEqual(result['status'],'failed')
        self.assertEqual(result['qualified_lifetimes'],2)
        for r in result['lifetimes']:self.assertAlmostEqual(r['heap']['slope_bytes_per_hour'],6000000)
    def test_evidence_growth_is_not_heap_growth(self):
        result=analyze([self.sample(i,1,1000000) for i in range(60)],1e9,60,1)
        self.assertEqual(result['status'],'passed')
        self.assertGreater(result['persistent_evidence']['slope_bytes_per_hour'],0)
    def test_missing_host_never_counts_as_zero_heap(self):
        rows=[self.sample(i,None,0) for i in range(60)]
        result=analyze(rows,1e9,60,1)
        self.assertEqual(result['status'],'failed')
        self.assertEqual(result['qualified_lifetimes'],0)
    def test_short_lifetime_is_not_a_passing_fit(self):
        result=analyze([self.sample(i,1,1000000) for i in range(5)],1e9,60,1)
        self.assertEqual(result['status'],'failed')

if __name__=='__main__':unittest.main()
