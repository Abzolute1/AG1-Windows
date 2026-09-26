import json
import sys

from . import Ledger
from .ledger import LedgerError


def main():
    if len(sys.argv) < 2:
        print("usage: python3 -m stockledger DATABASE < batch.json", file=sys.stderr)
        sys.exit(2)

    db_path = sys.argv[1]
    ledger = Ledger(db_path)
    try:
        batch = json.load(sys.stdin)
        result = ledger.apply_batch(batch)
        print(json.dumps(result))
        sys.exit(0)
    except LedgerError as exc:
        print(str(exc), file=sys.stderr)
        sys.exit(2)
    except (json.JSONDecodeError, ValueError) as exc:
        print(str(exc), file=sys.stderr)
        sys.exit(2)
    finally:
        ledger.close()


if __name__ == "__main__":
    main()
