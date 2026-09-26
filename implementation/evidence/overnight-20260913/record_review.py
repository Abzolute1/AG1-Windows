"""Bind explicitly supplied manual review decisions to immutable task evidence."""
import hashlib
import json
from pathlib import Path
import sys

project = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(project / 'tools'))
from release_gate import ref

root = Path(sys.argv[1])
(root / 'claim-reviews').mkdir(mode=0o700, exist_ok=True)
directories = sorted((root / 'activities').glob('*-task'))
for review in json.load(sys.stdin):
    index = review['task_index']
    directory = directories[index]
    assert (directory / 'finished.json').exists()
    assert review['status'] in ('passed', 'failed')
    assert len(review['rationale']) > 50 and review['claims']
    review['reviewer'] = 'Codex'
    for key in ('stale_authoritative_claims', 'unsupported_claims', 'unexpected_effects', 'unresolved_commands'):
        review.setdefault(key, [])
    review.setdefault('forbidden_avoidance', True)
    review.setdefault('contradiction_handling', True)
    for key in ('records', 'verification'):
        path = directory / (key + '.json')
        review[key] = ref(path.resolve())
    with (root / f'claim-reviews/{index:06}.json').open('x') as output:
        json.dump(review, output, indent=2)
    print(f'Saved supplied manual decision for task {index}: {review["status"]}')
