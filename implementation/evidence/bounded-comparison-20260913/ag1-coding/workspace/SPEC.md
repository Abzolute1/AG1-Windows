# Current contract, version 2
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
