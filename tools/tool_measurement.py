"""Measure only the owned P09 native fixture process tree; never start a desktop/model."""
from __future__ import annotations
import hashlib
import json
import math
from pathlib import Path
import subprocess
import tempfile
import time

from common import ROOT, GateError, read_json, sha256, write_json


def validate(data):
    if (data.get('status') != 'measured' or data.get('runtime') != 'rust'
        or data.get('samples') != 25 or not data.get('scope')
        or data.get('model_actions') != 0 or data.get('external_network_actions') != 0
        or data.get('no_effect_before_approval') is not True):
        raise GateError('Invalid tool measurement authority')
    for key in ['noop_prepare', 'durable_noop', 'sequential_reads', 'parallel_reads', 'serialized_mutations']:
        times(data.get(key, {}).get('ns'), 25)
    if data['noop_prepare'].get('calls_per_sample') != 1000:
        raise GateError('No-op batch changed')
    for key in ['sequential_reads', 'parallel_reads', 'serialized_mutations']:
        if data[key].get('calls_per_sample') != 8:
            raise GateError('File concurrency batch changed')
    if (data['parallel_reads'].get('peak_held_proposals') != 8
        or data['serialized_mutations'].get('peak_held_proposals') != 1
        or data['serialized_mutations'].get('final_bytes') != 399):
        raise GateError('File queue/effect measurement failed')
    text = ('abcdefghijklmnopqrstuvwxyz0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ!\n' * 512).encode()
    if (data.get('input_bytes') != len(text) or data.get('input_sha256') != hashlib.sha256(text).hexdigest()
        or len(data.get('read_result_sha256', '')) != 64):
        raise GateError('Changed tool measurement input/output')
    spool = data.get('large_spool', {})
    times(spool.get('ns'), 5)
    if (spool.get('bytes_per_sample') != 4194304 or spool.get('spools_cleaned') is not True
        or spool.get('sha256') != hashlib.sha256(b'x' * 4194304).hexdigest()):
        raise GateError('Incomplete large output custody')
    extension = data.get('extension', {})
    times(extension.get('roundtrip_ns'), 25)
    times([extension.get('startup_ns')], 1)
    if extension.get('stopped') is not True:
        raise GateError('Unsettled extension host')


def times(values, count):
    if not isinstance(values, list) or len(values) != count or any(type(v) is not int or v <= 0 for v in values):
        raise GateError('Incomplete timing samples')


def identity(pid):
    try:
        stat = Path(f'/proc/{pid}/stat').read_text().rsplit(')', 1)[1].split()
        # Field 22 start time prevents adoption of a reused numeric PID.
        return stat[19]
    except (FileNotFoundError, ProcessLookupError):
        return None


def process_sample(known):
    queue = list(known)
    seen, total, live = set(), 0, 0
    while queue:
        pid = queue.pop()
        if pid in seen or identity(pid) != known[pid]:
            continue
        seen.add(pid)
        try:
            status = Path(f'/proc/{pid}/status').read_text()
            for line in status.splitlines():
                if line.startswith('VmRSS:'):
                    total += int(line.split()[1])
                    live += 1
                    break
            for child in Path(f'/proc/{pid}/task/{pid}/children').read_text().split():
                child = int(child)
                stamp = identity(child)
                if stamp is not None:
                    known.setdefault(child, stamp)
                    queue.append(child)
        except (FileNotFoundError, ProcessLookupError):
            continue
    return total, live


def measure(binary, evidence, env):
    evidence.mkdir(parents=True, exist_ok=True)
    binary_hash = sha256(binary)
    reports = []
    for repeat in range(3):
        with tempfile.TemporaryDirectory(prefix='p09-measure-', dir=ROOT / '.local') as tmp:
            output = Path(tmp) / 'native.json'
            log = evidence / f'measurement-{repeat}.log'
            argv = [str(binary), tmp, str(output)]
            with log.open('w') as stream:
                start = time.monotonic()
                process = subprocess.Popen(argv, cwd=ROOT, env=env, stdout=stream, stderr=subprocess.STDOUT)
                known = {process.pid: identity(process.pid)}
                peak, processes, samples = 0, 0, 0
                try:
                    while process.poll() is None:
                        rss, live = process_sample(known)
                        peak, processes = max(peak, rss), max(processes, live)
                        samples += 1
                        if time.monotonic() - start > 90:
                            raise GateError('Tool measurement exceeded its fixture deadline')
                        time.sleep(.005)
                    if process.returncode:
                        raise GateError(f'Tool measurement failed; see {log}')
                finally:
                    if process.poll() is None:
                        process.kill()
                    process.wait()
            native = read_json(output)
            validate(native)
            if samples < 10 or processes < 2 or not 0 < peak <= 512 * 1024:
                raise GateError('Missing or excessive owned process-tree memory observations')
            reports.append({'native': native, 'argv': argv, 'exit_code': process.returncode,
                'seconds': time.monotonic() - start, 'observed_tree_peak_rss_kib': peak,
                'observed_peak_processes': processes, 'memory_samples': samples,
                'log': log.relative_to(ROOT).as_posix()})
    if sha256(binary) != binary_hash:
        raise GateError('Tool measurement executable changed')
    if len({r['native']['read_result_sha256'] for r in reports}) != 1:
        raise GateError('Repeated file measurement output changed')
    summary = {}
    for key in ['noop_prepare', 'durable_noop', 'sequential_reads', 'parallel_reads', 'serialized_mutations', 'large_spool']:
        values = sorted(v for row in reports for v in row['native'][key]['ns'])
        summary[key] = {'samples': len(values), 'median_ns': values[len(values)//2], 'p95_ns': values[math.ceil(len(values)*.95)-1]}
    result = {'status': 'measured', 'binary_sha256': binary_hash, 'runs': reports,
        'summary': summary,
        'scope': 'Three owned Linux synthetic runs. No TS speedup, application responsiveness, model throughput or other-platform claim. RSS is a 5ms sampled sum of the native fixture and observed descendants; it is not an allocator bound and may miss shorter-lived processes. Configured quotas and separate rejection tests establish admission bounds.',
        'model_actions': 0, 'external_network_actions': 0}
    write_json(evidence / 'performance.json', result)
    return result


if __name__ == '__main__':
    from tool_gate import environment
    _, env = environment()
    measure(ROOT / 'product/target/release/examples/tool-probe', ROOT / 'implementation/evidence/P09', env)
