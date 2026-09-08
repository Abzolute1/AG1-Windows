"""Fit observed native lifetimes separately so restart resets cannot hide growth."""
from collections import defaultdict
import math


def fit(rows, field):
    if len(rows) < 2:
        raise ValueError('At least two observations are required')
    origin = rows[0]['monotonic_ns']
    xs = [(r['monotonic_ns'] - origin) / 1e9 for r in rows]
    ys = [r[field] for r in rows]
    if any(type(y) not in (int, float) or not math.isfinite(y) or y < 0 for y in ys):
        raise ValueError('Invalid memory observation')
    xbar, ybar = sum(xs) / len(xs), sum(ys) / len(ys)
    xx = sum((x - xbar) ** 2 for x in xs)
    if xx <= 0:
        raise ValueError('Memory observations need distinct monotonic times')
    slope = sum((x-xbar)*(y-ybar) for x,y in zip(xs,ys)) / xx
    residual = sum((y-ybar-slope*(x-xbar))**2 for x,y in zip(xs,ys))
    stderr = math.sqrt(residual / (len(xs)-2) / xx) if len(xs)>2 else None
    return {'samples': len(rows), 'span_seconds': xs[-1], 'slope_bytes_per_hour': slope*3600,
            'slope_standard_error_bytes_per_hour': None if stderr is None else stderr*3600,
            'minimum_bytes': min(ys), 'maximum_bytes': max(ys),
            'start_monotonic_ns': origin, 'end_monotonic_ns': rows[-1]['monotonic_ns']}


def analyze(samples, start, warmup, budget):
    groups = defaultdict(list)
    absent = 0
    for sample in samples:
        if not sample.get('host_present'):
            absent += 1
            continue
        heap = sample.get('heap_observation')
        roots = [r for r in sample['processes'] if r['pid'] == heap['pid']] if heap else []
        if len(roots) != 1:
            raise ValueError('Allocator sample does not identify its observed native host')
        root = roots[0]
        key = (root['pid'], root['start_ticks'], root['executable_device'], root['executable_inode'])
        groups[key].append(sample)
    lifetimes = []
    qualified = []
    for key, rows in groups.items():
        # Both process startup and workload startup receive the frozen warmup.
        selected = [r for r in rows if r['monotonic_ns'] >= max(start, rows[0]['monotonic_ns']) + warmup*1e9]
        row = {'identity': list(key), 'samples': len(rows),
               'observed_seconds': (rows[-1]['monotonic_ns']-rows[0]['monotonic_ns'])/1e9,
               'post_warmup_samples': len(selected), 'qualified': len(selected)>=10}
        if row['qualified']:
            row['heap'] = fit(selected, 'heap_bytes')
            row['tree_pss'] = fit(selected, 'pss_bytes')
            row['passed'] = row['heap']['slope_bytes_per_hour'] <= budget
            qualified.append(row)
        else:
            row['reason'] = 'Too short for ten post-warmup samples; retained, never treated as a zero-growth lifetime.'
        lifetimes.append(row)
    persistent = fit(samples, 'persistent_evidence_bytes')
    return {'method': 'OLS per native process identity after frozen warmup; host absence and restarts never enter as zero heap samples',
            'heap_measurement': 'glibc in-use-or-retained estimate including allocator metadata and thread caches',
            'warmup_seconds_per_lifetime': warmup, 'heap_growth_budget_bytes_per_hour': budget,
            'host_absent_samples': absent, 'lifetimes': lifetimes, 'qualified_lifetimes': len(qualified),
            'persistent_evidence': persistent,
            'status': 'passed' if qualified and all(r['passed'] for r in qualified) else 'failed'}
