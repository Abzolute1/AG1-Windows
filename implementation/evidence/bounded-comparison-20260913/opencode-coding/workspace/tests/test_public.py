import tempfile, unittest
from pathlib import Path
from stockledger import Ledger, LedgerError

class Public(unittest.TestCase):
    def setUp(self):
        self.tmp=tempfile.TemporaryDirectory();self.path=Path(self.tmp.name)/'stock.db';self.ledger=Ledger(self.path)
    def tearDown(self):
        self.ledger.close();self.tmp.cleanup()
    def test_basic(self):
        self.assertEqual(self.ledger.apply_batch([{'id':'1','sku':'a','delta':4},{'id':'2','sku':'a','delta':-1}]),{'a':3})
    def test_replay(self):
        e={'id':'1','sku':'a','delta':4};self.ledger.apply_batch([e]);self.assertEqual(self.ledger.apply_batch([e]),{'a':4})
    def test_rollback(self):
        with self.assertRaises(LedgerError):self.ledger.apply_batch([{'id':'1','sku':'a','delta':2},{'id':'2','sku':'a','delta':-3}])
        self.assertEqual(self.ledger.stock(),{})
    def test_conflict(self):
        self.ledger.apply_batch([{'id':'1','sku':'a','delta':2}])
        with self.assertRaises(LedgerError):self.ledger.apply_batch([{'id':'1','sku':'a','delta':3}])
    def test_boolean(self):
        with self.assertRaises(LedgerError):self.ledger.apply_batch([{'id':'1','sku':'a','delta':True}])
