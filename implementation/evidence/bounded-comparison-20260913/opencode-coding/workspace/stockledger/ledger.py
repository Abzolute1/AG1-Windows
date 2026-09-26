import sqlite3


class LedgerError(ValueError):
    pass


def _validate_event(event):
    """Validate a single event dict. Raises LedgerError on any violation."""
    if not isinstance(event, dict):
        raise LedgerError("event must be a dict")

    # All events must have exactly the three required keys (plus possibly unknown)
    # id and sku are required
    if "id" not in event:
        raise LedgerError("missing field: id")
    if "sku" not in event:
        raise LedgerError("missing field: sku")
    if "delta" not in event:
        raise LedgerError("missing field: delta")

    eid = event["id"]
    sku = event["sku"]
    delta = event["delta"]

    # id: must be a nonempty string with at least one non-whitespace char
    if not isinstance(eid, str) or len(eid) == 0:
        raise LedgerError("id must be a nonempty string")
    if not any(not c.isspace() for c in eid):
        raise LedgerError("id must contain a non-whitespace character")

    # sku: same rules as id
    if not isinstance(sku, str) or len(sku) == 0:
        raise LedgerError("sku must be a nonempty string")
    if not any(not c.isspace() for c in sku):
        raise LedgerError("sku must contain a non-whitespace character")

    # delta: must be an integer, must NOT be a bool, must be nonzero
    if isinstance(delta, bool) or not isinstance(delta, int):
        raise LedgerError("delta must be a nonzero integer")
    if delta == 0:
        raise LedgerError("delta must be nonzero")


class Ledger:
    def __init__(self, path):
        self.path = path
        self.db = sqlite3.connect(path)
        self.db.execute(
            "CREATE TABLE IF NOT EXISTS events "
            "(id TEXT PRIMARY KEY, sku TEXT, delta INTEGER)"
        )
        self.db.commit()

    def stock(self):
        """Return the current stock map with alphabetically ordered keys."""
        rows = self.db.execute(
            "SELECT sku, SUM(delta) FROM events GROUP BY sku"
        ).fetchall()
        return dict(sorted((sku, total) for sku, total in rows))

    def apply_batch(self, events):
        """Validate every event first, then apply atomically.

        - Idempotent replay of same id+sku+delta is a no-op.
        - Reusing an id with different data raises LedgerError (atomic rollback).
        - Negative stock at any point within the batch raises LedgerError (atomic rollback).
        - Unknown/skipped (idempotent) events do not participate in stock checks.
        """
        if not isinstance(events, list):
            raise LedgerError("batch must be a list")

        # Phase 1: validate all events
        validated = []
        for event in events:
            _validate_event(event)
            validated.append(event)

        # Phase 2: check for id conflicts and idempotent replays
        # Load existing events that appear in this batch
        existing_map = {}  # id -> (sku, delta) for ids already in DB
        for eid in (e["id"] for e in validated):
            row = self.db.execute(
                "SELECT sku, delta FROM events WHERE id = ?", (eid,)
            ).fetchone()
            if row is not None:
                existing_map[eid] = (row[0], row[1])

        # Check for conflicts among new events and against existing data
        seen_ids = {}  # id -> event (within this batch)
        for event in validated:
            eid = event["id"]
            new_sku = event["sku"]
            new_delta = event["delta"]

            if eid in existing_map:
                old_sku, old_delta = existing_map[eid]
                if old_sku != new_sku or old_delta != new_delta:
                    raise LedgerError(
                        f"id '{eid}' reused with different data"
                    )
                # Idempotent replay: same data, no-op. Don't apply again.

            if eid in seen_ids:
                # Duplicate id within same batch
                prev = seen_ids[eid]
                if prev["sku"] != new_sku or prev["delta"] != new_delta:
                    raise LedgerError(
                        f"duplicate id '{eid}' with different data in batch"
                    )
                # Duplicate id with same data: no-op
                seen_ids[eid] = event  # keep reference
                continue

            seen_ids[eid] = event

        # Phase 3: compute running stock deltas and check for negative stock
        # We need to compute the running total per sku as events are applied in order
        # Only apply non-idempotent events (those not already in DB)
        # Track which new ids we're actually inserting
        running_stock = {}
        new_inserts = []  # events to actually insert (non-idempotent)

        # Initialize running_stock with current database values
        for sku, total in self.stock().items():
            running_stock[sku] = total

        inserted_ids = set()
        for event in validated:
            eid = event["id"]
            sku = event["sku"]
            delta = event["delta"]

            # Skip idempotent replays (already in DB with same data)
            if eid in existing_map:
                continue

            # Skip duplicates within this batch (same id already inserted)
            if eid in inserted_ids:
                continue

            # Check negative stock before applying
            current = running_stock.get(sku, 0)
            if current + delta < 0:
                raise LedgerError(
                    f"negative stock for sku '{sku}': {current} + {delta}"
                )
            running_stock[sku] = current + delta
            new_inserts.append(event)
            inserted_ids.add(eid)

        # Phase 4: insert all new events atomically
        if new_inserts:
            self.db.executemany(
                "INSERT INTO events (id, sku, delta) VALUES (?, ?, ?)",
                [(e["id"], e["sku"], e["delta"]) for e in new_inserts],
            )
        self.db.commit()

        return self.stock()

    def close(self):
        self.db.close()
