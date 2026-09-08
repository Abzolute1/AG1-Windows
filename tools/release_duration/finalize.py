"""Accept a duration only after every retained task has an explicit claim review."""
import argparse,json
from pathlib import Path
from run_duration import ROOT
from faults import retain
from memory_analysis import analyze
from release_gate import artifact,ref,input_digest,validate_report


def finalize(output):
    raw=output/'duration-result.json';data=json.load(open(raw))
    if data['status']!='awaiting-claim-review' or data['error'] is not None:
        raise RuntimeError('Only a fully executed duration can enter claim review acceptance')
    if data['input_sha256']!=input_digest():raise RuntimeError('Current release inputs changed after duration')
    tasks=data['tasks'];reviewed=[]
    for task in tasks:
        record=artifact(task['records']);verification=artifact(task['verification'])
        observed=json.load(open(verification))
        if observed['status']!='passed':raise RuntimeError('Original task verifier failed')
        review_path=output/'claim-reviews'/f'{task["index"]:06}.json'
        review=json.load(open(review_path))
        if review.get('status')!='passed' or review.get('records')!=ref(record) or review.get('verification')!=ref(verification):
            raise RuntimeError('Missing, stale or failed task claim review')
        if review.get('reviewer')!='Codex' or not review.get('rationale') or not review.get('claims'):
            raise RuntimeError('Claim review requires explicit analysis of actual assistant output and tools')
        for key in ('stale_authoritative_claims','unsupported_claims','unexpected_effects','unresolved_commands'):
            if review.get(key)!=[]:raise RuntimeError('Unresolved claim or execution review: '+key)
        for key in ('forbidden_avoidance','contradiction_handling'):
            if review.get(key) is not True:raise RuntimeError('Missing contract review: '+key)
        reviewed.append(ref(review_path))
    if len(list((output/'claim-reviews').glob('*.json')))!=len(tasks):raise RuntimeError('Claim review task coverage is not exact')
    memory=analyze(data['samples'],data['start_monotonic_ns'],data['warmup_seconds'],data['heap_growth_budget_bytes_per_hour'])
    if memory['status']!='passed':raise RuntimeError('Native lifetime heap fit failed')
    retained_memory=json.load(open(artifact(data['memory_analysis'])))
    if retained_memory!=memory:raise RuntimeError('Retained allocator analysis differs from raw samples')
    # Execution checks already require exact receipts, unchanged source/verifiers,
    # owned child settlement and a fresh read-only runtime preservation audit.
    # The explicit reviews above cover semantic claims and unexpected tool effects.
    total=len(tasks)
    data.update(status='passed',quality_review_pending=False,release_acceptance=False,
                claim_reviews=reviewed,raw_duration_result=ref(raw),
                quality={**{key:{'passed':total,'total':total} for key in
                ('required_recall','forbidden_avoidance','contradiction_handling','task_success')},
                **{key:0 for key in ('stale_authoritative_claims','leaked_owned_children','lost_commands','replayed_effects','runtime_preservation_violations')}})
    inventory={}
    for path in sorted(output.rglob('*')):
        if path.is_symlink():continue
        if path.is_file():inventory[str(path.relative_to(ROOT))]=ref(path)
    start=json.load(open(artifact(data['start_receipt'])))
    for row in start['bound_inputs']:
        artifact(row);inventory[row['path']]=row
    data['artifacts']=list(inventory.values())
    package=json.load(open(ROOT/'implementation/evidence/P14/package.json'))
    validate_report(data['kind'],data,input_digest(),package['archive']['sha256'])
    destination=output/'accepted-duration.json';retain(destination,data)
    return destination

if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('output',type=Path)
    print(finalize(p.parse_args().output.absolute()))
