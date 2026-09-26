import contextlib, hashlib, importlib, io, json, subprocess, sys, tempfile, unittest
from pathlib import Path

PROJECT=Path(sys.argv[1]).resolve();FOLLOWUP='--followup' in sys.argv
sys.path.insert(0,str(PROJECT))
from stockledger import Ledger, LedgerError
def event(i='1',sku='a',delta=3):return {'id':i,'sku':sku,'delta':delta}
class Contract(unittest.TestCase):
 def setUp(self):
  self.temp=tempfile.TemporaryDirectory();self.path=Path(self.temp.name)/'db';self.l=Ledger(self.path)
 def tearDown(self):self.l.close();self.temp.cleanup()
 def test_normal_ordered_zero_stock(self):
  self.assertEqual(self.l.apply_batch([event('1','z',4),event('2','a',2),event('3','z',-4)]),{'a':2,'z':0});self.assertEqual(list(self.l.stock()),['a','z'])
 def test_replay_same_batch(self):
  e=event();self.assertEqual(self.l.apply_batch([e,e,e]),{'a':3})
 def test_replay_after_reopen(self):
  self.l.apply_batch([event()]);self.l.close();self.l=Ledger(self.path);self.assertEqual(self.l.apply_batch([event()]),{'a':3})
 def test_conflict_delta_atomic(self):
  self.l.apply_batch([event()])
  with self.assertRaises(LedgerError):self.l.apply_batch([event('2','b',2),event(delta=4)])
  self.assertEqual(self.l.stock(),{'a':3});self.assertEqual(self.l.apply_batch([event('2','b',5)]),{'a':3,'b':5})
 def test_conflict_sku(self):
  with self.assertRaises(LedgerError):self.l.apply_batch([event(),event(sku='b')])
  self.assertEqual(self.l.stock(),{})
 def test_negative_intermediate_not_just_final(self):
  with self.assertRaises(LedgerError):self.l.apply_batch([event('1','a',-1),event('2','a',3)])
  self.assertEqual(self.l.stock(),{})
 def test_failed_batch_preserves_previous_commit(self):
  self.l.apply_batch([event()])
  with self.assertRaises(LedgerError):self.l.apply_batch([event('2','a',2),event('3','a',-6)])
  self.l.close();self.l=Ledger(self.path);self.assertEqual(self.l.stock(),{'a':3})
 def test_invalid_last_event_atomic(self):
  with self.assertRaises(LedgerError):self.l.apply_batch([event(),{'id':'last','sku':'b','delta':False}])
  self.assertEqual(self.l.stock(),{})
 def test_delta_types(self):
  for d in [0,True,False,1.0,'2',None,[],{}]:
   with self.subTest(delta=d),self.assertRaises(LedgerError):self.l.apply_batch([event(delta=d)])
  self.assertEqual(self.l.stock(),{})
 def test_id_sku_types_and_whitespace(self):
  for field in ['id','sku']:
   for value in ['', '  \t', 5, True, None, [],{}]:
    e=event();e[field]=value
    with self.subTest(field=field,value=value),self.assertRaises(LedgerError):self.l.apply_batch([e])
 def test_missing_extra_and_nonobjects(self):
  for e in [None,[],1,'x',{'id':'1','sku':'a'},dict(event(),extra=1)]:
   with self.subTest(event=e),self.assertRaises(LedgerError):self.l.apply_batch([e])
 def test_batch_is_list(self):
  for batch in [None,{},'abc',(event(),),iter([event()])]:
   with self.subTest(batch=type(batch).__name__),self.assertRaises(LedgerError):self.l.apply_batch(batch)
 def test_preserve_case_and_spaces(self):
  result=self.l.apply_batch([event('1','A',2),event('2','a',3),event(' 1 ',' a ',4)])
  self.assertEqual(result,{' a ':4,'A':2,'a':3});self.assertEqual(list(result),[' a ','A','a'])
 def test_empty_and_two_connections(self):
  other=Ledger(self.path)
  try:
   self.l.apply_batch([event()]);self.assertEqual(other.apply_batch([]),{'a':3});other.apply_batch([event('2','a',2)]);self.assertEqual(self.l.stock(),{'a':5})
  finally:other.close()
 def test_input_is_not_modified(self):
  original=[event()];copy=json.loads(json.dumps(original));self.l.apply_batch(original);self.assertEqual(original,copy)
 def cli(self,body):return subprocess.run([sys.executable,'-m','stockledger',str(self.path)],input=body,text=True,capture_output=True,cwd=PROJECT,timeout=10)
 def test_cli_success_sorted(self):
  r=self.cli(json.dumps([event('1','z',1),event('2','a',2)]));self.assertEqual(r.returncode,0,r.stderr);self.assertEqual(json.loads(r.stdout),{'a':2,'z':1});self.assertEqual(list(json.loads(r.stdout)),['a','z']);self.assertEqual(r.stderr,'')
 def test_cli_bad_json(self):
  r=self.cli('{bad');self.assertEqual(r.returncode,2);self.assertEqual(r.stdout,'');self.assertTrue(r.stderr.strip());self.assertNotIn('Traceback',r.stderr)
 def test_cli_contract_failure_is_atomic(self):
  r=self.cli(json.dumps([event(),event('2','a',-4)]));self.assertEqual(r.returncode,2);self.assertEqual(r.stdout,'');self.assertNotIn('Traceback',r.stderr);self.assertEqual(self.l.stock(),{})
class DryRun(Contract):
 def test_dry_success_then_reuse_id_differently(self):
  self.l.apply_batch([event()]);self.assertEqual(self.l.apply_batch([event('2','b',4)],dry_run=True),{'a':3,'b':4});self.assertEqual(self.l.stock(),{'a':3});self.assertEqual(self.l.apply_batch([event('2','c',2)]),{'a':3,'c':2})
 def test_dry_failure_does_not_leak_ids(self):
  with self.assertRaises(LedgerError):self.l.apply_batch([event(),event('2','a',-4)],dry_run=True)
  self.assertEqual(self.l.apply_batch([event(delta=6)]),{'a':6})
 def test_dry_replay_and_conflict(self):
  self.l.apply_batch([event()]);self.assertEqual(self.l.apply_batch([event(),event('2','a',-1)],dry_run=True),{'a':2})
  with self.assertRaises(LedgerError):self.l.apply_batch([event('3','b',1),event(delta=9)],dry_run=True)
  self.assertEqual(self.l.stock(),{'a':3})
 def test_dry_no_new_sku_after_reopen(self):
  self.l.apply_batch([event()],dry_run=True);self.l.close();self.l=Ledger(self.path);self.assertEqual(self.l.stock(),{})
 def test_dry_validation_and_order(self):
  for batch in [[event(delta=True)],[event('1','a',-1),event('2','a',3)]]:
   with self.assertRaises(LedgerError):self.l.apply_batch(batch,dry_run=True)
  self.assertEqual(self.l.stock(),{})
 def test_dry_parameter_keyword_only(self):
  with self.assertRaises(TypeError):self.l.apply_batch([event()],True)

suite=unittest.defaultTestLoader.loadTestsFromTestCase(DryRun if FOLLOWUP else Contract)
stream=io.StringIO();result=unittest.TextTestRunner(stream=stream,verbosity=2).run(suite)
manifest=json.loads((Path(__file__).parent/'seed-manifest.json').read_text())
protected={name:(PROJECT/name).exists() and hashlib.sha256((PROJECT/name).read_bytes()).hexdigest()==digest for name,digest in manifest.items() if name.startswith(('tests/','docs/')) or name in ['SPEC.md','USER-NOTES.txt']}
print(json.dumps({'tests':result.testsRun,'failures':len(result.failures),'errors':len(result.errors),'passed':result.wasSuccessful(),'protected':protected,'output':stream.getvalue()},indent=2))
