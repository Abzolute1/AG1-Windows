"""Matched P10 fake sessions; actual TS/Rust durability costs remain explicit."""
import hashlib
import math
from pathlib import Path
import subprocess
import tempfile
import time
from common import ROOT, GateError, read_json, write_json, sha256, isolated_env
from tool_measurement import identity, process_sample

METRICS = ['total_ns', 'first_request_ns', 'tool_context_ns', 'followup_ns']

def validate(data, runtime):
    if (data.get('status') != 'measured' or data.get('runtime') != runtime or not data.get('scope')
        or data.get('model_actions') != 0 or data.get('external_network_actions') != 0):
        raise GateError('Invalid runtime measurement authority')
    digest = hashlib.sha256(('Owned evidence β\n' * 256).encode()).hexdigest()
    if data.get('input_sha256') != digest or len(data.get('samples', [])) != 25 or len(data.get('cycles', [])) != 32:
        raise GateError('Missing or changed runtime measurement workload')
    for sample in data['samples']:
        if (sample.get('requests') != 3 or sample.get('tool_results') != 1 or sample.get('full_read_evidence') is not True
            or sample.get('users') != ['Read input.txt', 'Continue']):
            raise GateError('Runtime measurement lost tool/context/queue correctness')
        if any(type(sample.get(k)) is not int or sample[k] <= 0 for k in METRICS):
            raise GateError('Missing runtime timing samples')
    for i, cycle in enumerate(data['cycles']):
        if (cycle.get('cycle') != i or cycle.get('journal_entries') != 6 + 4*i
            or cycle.get('projected_messages') != 2 or type(cycle.get('ns')) is not int or cycle['ns'] <= 0
            or type(cycle.get('heap_bytes')) is not int or not 0 < cycle['heap_bytes'] <= 128*1024*1024):
            raise GateError('Resume/compaction lost history, projection bound or heap observations')
    heaps = [r['heap_bytes'] for r in data['cycles']]
    if max(heaps[-8:]) - min(heaps[:8]) > 16*1024*1024:
        raise GateError('Excessive heap growth over the retained 32-cycle workload')

def percentile(values):
    return sorted(values)[math.ceil(len(values)*.95)-1]

def measure(binary, evidence, node):
    reports = []
    binary_hash = sha256(binary)
    with tempfile.TemporaryDirectory(prefix='ag1-p10-measure-') as directory:
        env = isolated_env(Path(directory))
        env['PATH'] = str(node.parent) + ':' + env['PATH']
        env['GIT_CEILING_DIRECTORIES'] = directory
        for i in range(3):
            for runtime in (['rust','typescript'] if i % 2 == 0 else ['typescript','rust']):
                name = f'measure-{runtime}-{i}'
                output = evidence / (name + '.json')
                output.unlink(missing_ok=True)
                argv = ([str(binary),str(output)] if runtime == 'rust' else
                        [str(node),'--expose-gc','tools/probes/runtime_measure.mjs',str(output)])
                log = evidence / (name + '.log')
                start = time.monotonic()
                peak = observations = 0
                with log.open('w') as stream:
                    process = subprocess.Popen(argv,cwd=ROOT,env=env,stdout=stream,stderr=subprocess.STDOUT)
                    known = {process.pid: identity(process.pid)}
                    try:
                        while process.poll() is None:
                            rss,_ = process_sample(known)
                            peak = max(peak,rss);observations += 1
                            if time.monotonic()-start > 120:
                                raise GateError('Runtime measurement deadline exceeded')
                            time.sleep(.005)
                        if process.returncode:
                            raise GateError(f'Runtime measurement failed: {log}')
                    finally:
                        if process.poll() is None: process.kill()
                        process.wait()
                data = read_json(output);validate(data,runtime)
                if observations < 10 or not 0 < peak <= 512*1024:
                    raise GateError('Missing or excessive runtime process memory')
                reports.append({'runtime':runtime,'data':data,'argv':argv,'exit_code':0,
                    'seconds':time.monotonic()-start,'log':log.relative_to(ROOT).as_posix(),
                    'tree_peak_rss_kib':peak,'memory_samples':observations})
    if sha256(binary) != binary_hash: raise GateError('Runtime measurement binary changed')
    summary = {}
    for metric in METRICS:
        values = {runtime:[s[metric] for r in reports if r['runtime']==runtime for s in r['data']['samples']]
                  for runtime in ['rust','typescript']}
        summary[metric] = {runtime:{'median_ns':sorted(v)[len(v)//2],'p95_ns':percentile(v)} for runtime,v in values.items()}
        summary[metric]['rust_to_ts_p95_ratio'] = percentile(values['rust']) / percentile(values['typescript'])
    result = {'status':'measured','runs':reports,'summary':summary,'binary_sha256':binary_hash,
        'scope':'Three alternating-order Linux runs, 25 samples and 32 cumulative resume/compaction cycles each. Identical input bytes, three requests, one real read result and ordered follow-up; implementations retain their own prompts, evidence and storage semantics. Rust FULL SQL and original request/result custody differ from TS JSONL. Ratios describe these delivered host costs, not a matched pure-core regression or whole-product/model speedup. Heap is glibc allocated bytes versus post-GC V8 heapUsed; RSS includes observed fixture descendants at 5ms intervals. The 128 MiB heap, 16 MiB growth and two-message projection checks bound this finite workload; physical history intentionally grows. Release 8h/24h and live per-model acceptance remain P14.',
        'model_actions':0,'external_network_actions':0}
    write_json(evidence/'performance.json',result)
    return result
