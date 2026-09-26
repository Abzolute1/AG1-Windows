"""Stop new endurance activity after an explicit, bound failed quality review."""
import json
from pathlib import Path

from release_gate import artifact


def reject_failed_reviews(output: Path, tasks, *, allow_semantic_continuation=False):
    for task in tasks:
        path = output / 'claim-reviews' / f'{task["index"]:06}.json'
        if path.is_symlink():
            raise RuntimeError('Duration review cannot be a symlink: ' + str(path))
        if not path.exists():
            continue
        if path.stat().st_size > 1024 * 1024:
            raise RuntimeError('Duration review exceeds its bounded size: ' + str(path))
        try:
            review = json.loads(path.read_text())
        except json.JSONDecodeError:
            # An asynchronous reviewer may still be publishing this file.
            # Finalization independently requires complete valid passed reviews.
            continue
        if not isinstance(review, dict) or review.get('status') != 'failed':
            continue
        if (review.get('reviewer') != 'Codex'
                or review.get('task_index') != task['index']
                or review.get('records') != task['records']
                or review.get('verification') != task['verification']
                or not review.get('rationale')):
            raise RuntimeError('Failed review does not bind the completed task: ' + str(path))
        artifact(task['records'])
        artifact(task['verification'])
        # A user-directed investigation can retain a reviewed semantic failure
        # and continue measuring endurance. Formal release runs stay fail-fast;
        # unexpected effects or unresolved commands always stop either mode.
        if (allow_semantic_continuation
                and review.get('investigation_continuation') == 'semantic-only'
                and review.get('unexpected_effects') == []
                and review.get('unresolved_commands') == []
                and review.get('forbidden_avoidance') is True):
            continue
        raise RuntimeError('Quality review failed; no further duration activity: ' + str(path))
