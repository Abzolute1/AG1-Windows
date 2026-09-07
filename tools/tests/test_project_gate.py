import copy
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import sys
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from common import GateError
import project_gate as gate

class ProjectGateTests(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory();self.root=Path(self.temp.name)
        for name in gate.MEASUREMENTS:
            value={'commands':[['fixture']],'logs':['fixture.log'],'source_revision':'current','elapsed_ms':1,'lifecycle':'exited','exits':[0]}
            if name=='web-ready':value.update(lifecycle='ready',ready_ms=1,first_log_ms=1,target_rss_bytes=10)
            if name=='web-stopped':value.update(lifecycle='stopped',stop_ms=1)
            if name=='tauri-crash':value.update(lifecycle='failed',exits=[134])
            self.write(name+'.json',value)
        self.write('browser.json',{'browser_rss_bytes':20,'target_rss_bytes':10,'total_rss_bytes':30,'observation':{'privileged':{'node':False,'tauri':False},'errors':['deliberate frontend runtime failure']}})
        self.write('interactive-webkit.json',{'status':'passed','observation':{'denied':True,'count':'1'},'blocked_navigation':True,'closed_preview':True})
        for name in ['browser.png','tauri.png','tauri-crash.png','interactive-webkit.png']:(self.root/name).write_bytes(b'\x89PNG\r\n\x1a\n'+bytes(101))
    def tearDown(self):self.temp.cleanup()
    def write(self,name,value):(self.root/name).write_text(json.dumps(value))
    def test_complete_native_measurements(self):self.assertIn('operations',gate.validate_measurements(self.root))
    def test_compiler_success_cannot_replace_native_runtime(self):
        (self.root/'tauri.png').unlink()
        with self.assertRaises(GateError):gate.validate_measurements(self.root)
    def test_missing_readiness_rejected(self):
        value=json.loads((self.root/'web-ready.json').read_text());value['ready_ms']=None;self.write('web-ready.json',value)
        with self.assertRaises(GateError):gate.validate_measurements(self.root)
    def test_browser_memory_must_be_in_total(self):
        value=json.loads((self.root/'browser.json').read_text());value['total_rss_bytes']=10;self.write('browser.json',value)
        with self.assertRaises(GateError):gate.validate_measurements(self.root)
    def test_preview_privileges_reject_acceptance(self):
        value=json.loads((self.root/'interactive-webkit.json').read_text());value['observation']['denied']=False;self.write('interactive-webkit.json',value)
        with self.assertRaises(GateError):gate.validate_measurements(self.root)
    def test_empty_duplicate_and_unexecuted_coverage(self):
        case={'path':'contract.rs','test':'actual'};value={'phase':'P11','rows':[{'id':i,'success':[case],'failure':[case]} for i in gate.REQUIRED]}
        gate.validate_coverage(value,{('contract.rs','actual')})
        for bad in [{**value,'rows':[]},{**value,'rows':value['rows']+[value['rows'][0]]}]:
            with self.assertRaises(GateError):gate.validate_coverage(bad,{('contract.rs','actual')})
        with self.assertRaises(GateError):gate.validate_coverage(value,set())
    def test_changed_tooling_is_not_silently_rebaselined(self):
        with patch.object(gate,'read_json',return_value={'xvfb':{'path':str(self.root/'missing'),'sha256':'0'*64}}):
            with self.assertRaises(GateError):gate.verify_tooling()
    def test_skipped_and_empty_native_selection(self):
        from provider_gate import validate_rust_log
        for log in ['test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out;', 'test result: ok. 1 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out;']:
            with self.assertRaises(GateError):validate_rust_log(log,1)
