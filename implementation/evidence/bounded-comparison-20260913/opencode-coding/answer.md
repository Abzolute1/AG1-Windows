Now I'll implement the complete fix. The current code has critical gaps: no validation, no atomicity, no idempotency, no conflict detection, no negative stock check, no sorted keys, and broken CLI error handling.



All 5 tests pass. Now let me run additional checks against SPEC requirements:



Bug with duplicate ids within same batch. Let me fix:



All 20 additional checks pass. Now let me test the CLI:



Here's the summary:

## Changes

**`stockledger/ledger.py`** — complete rewrite:

| Area | Before (broken) | After (fixed) |
|---|---|---|
| **Validation** | Only checked `isinstance(delta, int)` | Full validation: nonempty string id/sku with non-whitespace, nonzero int delta rejecting bools/strings/floats, missing fields |
| **Atomicity** | `COMMIT` after every individual event | 4-phase process: validate all → check conflicts → compute running stock → insert atomically via `executemany` + single `COMMIT` |
| **Idempotent replay** | `INSERT OR IGNORE` silently ignored duplicates | Exact replay of same id+sku+delta is a no-op; tracked across batches and within-batch |
| **Conflict detection** | None — `INSERT OR IGNORE` swallowed conflicts | Cross-batch and within-batch conflict detection: different sku/delta on same id raises `LedgerError` |
| **Negative stock** | None — allowed any delta | Running stock computed per-event in order; any negative result raises `LedgerError` and rolls back all new inserts |
| **Sorted keys** | `dict()` from unsorted cursor | `dict(sorted(...))` returns alphabetically ordered keys |
| **Cross-connection** | Uncommitted changes invisible | Single `COMMIT` at end of batch ensures separate `Ledger` instances see committed data |

**`stockledger/__main__.py`** — complete rewrite:

| Area | Before (broken) | After (fixed) |
|---|---|---|
| **Error output** | No error handling, traceback on crash | `LedgerError` / `JSONDecodeError` caught → message to stderr only |
| **Exit codes** | Always 0 | Exit 0 on success, exit 2 on error |
| **Traceback** | Uncaught exceptions produced tracebacks | All exceptions caught, no tracebacks |
| **Cleanup** | No close on error | `finally: ledger.close()` always runs |

## Validation Results

- **5/5** supplied tests pass (`test_basic`, `test_replay`, `test_rollback`, `test_conflict`, `test_boolean`)
- **20/20** additional SPEC compliance checks pass covering: empty batches, zero SKUs, sorted keys, within-batch dedup, zero-delta rejection, missing fields, empty/whitespace strings, non-string types, string/float deltas, cross-connection visibility, negative stock rollback, multi-SKU rollback, cross-batch persistence, boolean deltas, idempotent replay, CLI success/error/JSON-error cases

## Remaining Limitations

- `apply_batch` accepts any list — spec says "JSON-style list" but the API accepts Python lists directly (JSON parsing is the CLI's responsibility). Non-list inputs raise `LedgerError` as expected.
- The spec says "preserve original spelling, including surrounding spaces" — this is handled by storing the raw string values without modification.
- No migration/versioning of the SQLite schema — acceptable per spec.
