import copy
from pathlib import Path
import tempfile
import unittest
import sys
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from common import GateError,read_json,ROOT
import client_gate as gate

class ClientGateTests(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory();self.root=Path(self.temp.name)
        names=['home-light','home-dark','chat-light','chat-dark','dialog-light','files-dirty-light','files-conflict-light','plans-light','context-light','activity-light','runtime-light','terminal-light','approval-light','models-conflict-light','resnapshot-light']
        self.report={'status':'passed','viewport':{'width':1440,'height':900,'dpr':1},'cases':{name:{'passed':True} for name in gate.VISUAL},'latency':{'samples':100,'p95_ms':20},'screenshots':[{'name':name,'path':name+'.png'} for name in names]}
        self.report['cases']['frozen_renderer_resnapshot'].update(frozen_ms=30000,provider_completed_ms=6000)
        self.report['cases']['1000_message_scroll']['rendered_articles']=7
        for row in self.report['screenshots']:(self.root/row['path']).write_bytes(b'\x89PNG\r\n\x1a\n'+bytes(8)+bytes.fromhex('000005a000000384')+bytes(100))
    def tearDown(self):self.temp.cleanup()
    def test_complete_native_capture(self):gate.validate_visual(self.report,self.root)
    def test_latency_threshold_is_strict(self):
        for latency in [50,51,-1]:
            self.report['latency']['p95_ms']=latency
            with self.assertRaises(GateError):gate.validate_visual(self.report,self.root)
    def test_quick_run_cannot_certify_freeze(self):
        del self.report['cases']['frozen_renderer_resnapshot']
        with self.assertRaises(GateError):gate.validate_visual(self.report,self.root)
    def test_core_must_finish_while_renderer_is_frozen(self):
        self.report['cases']['frozen_renderer_resnapshot']['provider_completed_ms']=31000
        with self.assertRaises(GateError):gate.validate_visual(self.report,self.root)
    def test_missing_or_unmatched_screenshot_fails(self):
        path=self.root/self.report['screenshots'][0]['path'];path.unlink()
        with self.assertRaises(GateError):gate.validate_visual(self.report,self.root)
    def test_dispositions_must_be_complete_and_honest(self):
        value=read_json(ROOT/'fixtures/clients/dispositions.json');gate.validate_dispositions(value)
        value['rows'][-1]['disposition']='native-adapter'
        with self.assertRaises(GateError):gate.validate_dispositions(value)
        value['rows']=value['rows'][:-1]
        with self.assertRaises(GateError):gate.validate_dispositions(value)
    def test_coverage_requires_executed_success_and_failure(self):
        case={'path':'client.rs','test':'actual'};value={'phase':'P12','rows':[{'id':id,'success':[case],'failure':[case]} for id in gate.REQUIRED]}
        gate.validate_coverage(value,{('client.rs','actual')})
        with self.assertRaises(GateError):gate.validate_coverage(value,set())
        value['rows'][0]['failure']=[]
        with self.assertRaises(GateError):gate.validate_coverage(value,{('client.rs','actual')})
    def test_duplicate_requirement_fails(self):
        value={'phase':'P12','rows':[{'id':id} for id in gate.REQUIRED]};value['rows'].append(value['rows'][0])
        with self.assertRaises(GateError):gate.validate_coverage(value,set())
