import json
import sys

from . import Ledger, LedgerError


def main():
    if len(sys.argv) < 2:
        print("Usage: python3 -m stockledger DATABASE", file=sys.stderr)
        sys.exit(2)

    db_path = sys.argv[1]
    try:
        ledger = Ledger(db_path)
        batch = json.load(sys.stdin)
        result = ledger.apply_batch(batch)
        print(json.dumps(result))
        ledger.close()
    except LedgerError as e:
        print(str(e), file=sys.stderr)
        sys.exit(2)
    except json.JSONDecodeError as e:
        print(f"invalid JSON: {e}", file=sys.stderr)
        sys.exit(2)


if __name__ == "__main__":
    main()
