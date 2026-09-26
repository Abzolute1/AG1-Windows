import hashlib, json, shutil
from pathlib import Path

ROOT=Path('/home/alex/Projects/AG1')
E=Path(__file__).parent
W=ROOT/'.local/bounded-comparison-20260913'
files={
'README.md': '''# Stock ledger
Small Python 3 standard-library inventory service. The current contract is SPEC.md.
Run `python3 -m unittest discover -s tests -v` from this directory.
The historical notes are retained for context and are not the current contract.
''',
'SPEC.md': '''# Current contract, version 2
Public API: `from stockledger import Ledger, LedgerError`; `Ledger(path)` opens SQLite;
`apply_batch(events)` returns the complete stock map with alphabetically ordered keys;
`stock()` returns the same map; `close()` closes the database.
Each event is exactly an object containing id (nonempty string), sku (nonempty string),
and delta (nonzero integer; booleans are invalid). Unknown/missing fields are invalid.
Strings are case-sensitive and must contain a non-whitespace character. Preserve the
original spelling, including surrounding spaces. Validation errors raise LedgerError.
A batch is a JSON-style list. Validate every event before applying any change.
The batch is atomic: a later invalid event, id conflict or negative stock rolls back
ALL new events in that batch. Existing data survives failed batches and reopening.
Never allow stock below zero, even temporarily within a batch; apply events in order.
Replaying an id with exactly the same sku/delta is a no-op, including duplicates
inside one batch. Reusing an id with changed data is an error. Replay history persists.
New SKUs begin at zero. Include known SKUs at zero in stock(). Empty batches return
current stock. Separate Ledger connections to one database must see committed updates.
CLI: `python3 -m stockledger DATABASE` reads one JSON batch from stdin. On success
print only the resulting sorted JSON object and exit 0. On invalid input, print a
concise error to stderr, nothing to stdout, no traceback, and exit 2. Use the same API.
No third-party dependencies. Preserve tests/, SPEC.md, docs/, and USER-NOTES.txt.
''',
'USER-NOTES.txt':'User draft: keep the cobalt shipment reserved for Friday. Do not alter.\n',
'docs/legacy.md':'# Version 1 (obsolete)\nBatches once committed each event independently. Boolean quantities were once accepted. SPEC.md supersedes this behavior.\n',
'stockledger/__init__.py':'from .ledger import Ledger, LedgerError\n',
'stockledger/ledger.py': '''import sqlite3

class LedgerError(ValueError):
    pass

class Ledger:
    def __init__(self, path):
        self.db = sqlite3.connect(path)
        self.db.execute("CREATE TABLE IF NOT EXISTS events (id TEXT PRIMARY KEY, sku TEXT, delta INTEGER)")
        self.db.commit()

    def stock(self):
        return dict(self.db.execute("SELECT sku, SUM(delta) FROM events GROUP BY sku"))

    def apply_batch(self, events):
        for event in events:
            if not isinstance(event['delta'], int):
                raise LedgerError('delta must be an integer')
            self.db.execute("INSERT OR IGNORE INTO events VALUES (?, ?, ?)",
                            (event['id'], event['sku'], event['delta']))
            self.db.commit()
        return self.stock()

    def close(self):
        self.db.close()
''',
'stockledger/__main__.py': '''import json, sys
from . import Ledger

ledger = Ledger(sys.argv[1])
print(json.dumps(ledger.apply_batch(json.load(sys.stdin))))
''',
'tests/test_public.py': '''import tempfile, unittest
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
'''
}
seed=W/'seed';seed.mkdir()
for name,text in files.items():
    p=seed/name;p.parent.mkdir(parents=True,exist_ok=True);p.write_text(text)
for adapter in ['ag1','opencode']:
    p=W/adapter;p.mkdir();shutil.copytree(seed,p/'coding');(p/'web').mkdir()
web='''Research a production Python HTTP client retry policy using current official documentation. Use web search for discovery, then actually fetch and read at least three relevant official sources from distinct domains: docs.python.org, developer.mozilla.org, and urllib3.readthedocs.io. Make at least two distinct search queries and at most four; if search is empty or blocked, say so and use known official URLs instead. Do not keep retrying an empty search.

Answer this scenario: a POST that creates an order times out after its request body may have reached the server; another response is HTTP 429 with Retry-After; and we need an overall 12-second elapsed-time budget. Explain when automatic retries are unsafe, what an idempotency key changes, both forms of Retry-After and handling of a past date, why an elapsed-time deadline needs a monotonic clock, and urllib3 Retry's default allowed-method set and whether POST is in it. Explicitly check the claim "urllib3 retries POST by default" against its documentation. End with pseudocode for a bounded retry decision. Cite the source next to each factual claim, distinguish your proposed policy from documented defaults, and keep the answer below 700 words. Give brief progress updates between searches and reading sources. Do not create or modify files.'''
coding='''Repair this stock-ledger package against SPEC.md. Inspect the code and reproduce relevant failures, then implement the full contract across the API and CLI. Pay particular attention to atomic batches, durable idempotency, conflicting event IDs, validation, and negative stock. Use only Python's standard library. Do not modify tests/, SPEC.md, docs/, or USER-NOTES.txt. Run the supplied tests and meaningful additional checks without weakening expectations. Finish with a concise account of changes, actual validation results, and remaining limitations. Work only in this supplied project.'''
follow='''Extend the repaired API with apply_batch(events, *, dry_run=False). With dry_run=True, perform exactly the same validation, ordering, replay/conflict and nonnegative-stock checks, and return the predicted complete sorted stock map, but commit neither stock nor new event IDs. A successful dry run must allow the same IDs to be used later for different real events. A failed dry run must also leave persistent state unchanged. Existing calls and CLI behavior must stay compatible. Keep the earlier protected files unchanged. Implement this, verify both dry-run and original behavior, and report what actually passed.'''
plan={'scope':'Bounded sequential local-model comparison; not endurance/release acceptance','model':'Qwen3.6-35B-A3B-UD-Q4_K_XL','order':['AG1 web/coding/followup','stop AG1 and verify GPU runtime gone','OpenCode web/coding/followup'],'cases':[{'id':'web','prompt':web,'limit_seconds':240},{'id':'coding','prompt':coding,'limit_seconds':360},{'id':'followup','prompt':follow,'limit_seconds':240}], 'scoring':{'coding':'held-out API/CLI contract tests plus protected-file integrity','followup':'held-out dry-run tests plus original suite','web':'actual search/fetch evidence, official-source coverage, accuracy on seven stated criteria, citation usefulness, honesty about search failure','visual':'actual native screenshot inspection and icon load/chronology/overflow measurements'},'limits':'One attempt per scored stage; failures retained. Setup mistakes corrected separately. Native toolsets/system prompts and search providers differ; no universal winner claim.'}
(E/'plan.json').write_text(json.dumps(plan,indent=2)+'\n')
(E/'seed-manifest.json').write_text(json.dumps({k:hashlib.sha256(v.encode()).hexdigest() for k,v in files.items()},indent=2)+'\n')
print('Prepared identical tasks and projects')
