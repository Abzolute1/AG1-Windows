"""Retain workstation contention observations around an explicitly selected measurement.

This is an observation wrapper, not a performance acceptance scorer. It does not
change benchmark inputs, evict caches, stop other applications or retry a run.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import threading
import time

ROOT = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(ROOT / 'tools'))
from release_gate import ref


def snapshot():
    processes = {}
    for path in Path('/proc').iterdir():
        if not path.name.isdigit():
            continue
        try:
            raw = (path / 'stat').read_text()
            fields = raw.rsplit(')', 1)[1].split()
            processes[int(path.name)] = {
                'pid': int(path.name), 'ppid': int(fields[1]),
                'start_ticks': int(fields[19]),
                'comm': raw.split('(', 1)[1].rsplit(')', 1)[0],
                'cpu_ticks': int(fields[11]) + int(fields[12]),
            }
        except (FileNotFoundError, ProcessLookupError, PermissionError):
            continue
    cpu = list(map(int, Path('/proc/stat').read_text().splitlines()[0].split()[1:9]))
    return {'monotonic_ns': time.monotonic_ns(), 'cpu': cpu,
            'loadavg': os.getloadavg(), 'processes': processes,
            'io_pressure': Path('/proc/pressure/io').read_text(),
            'memory_pressure': Path('/proc/pressure/memory').read_text()}


def interval(before, after, leader):
    elapsed = (after['monotonic_ns'] - before['monotonic_ns']) / 1e9
    owned = {leader, os.getpid()}
    while True:
        expanded = owned | {pid for pid, row in after['processes'].items()
                            if row['ppid'] in owned}
        if expanded == owned:
            break
        owned = expanded
    active = []
    for pid, row in after['processes'].items():
        old = before['processes'].get(pid)
        if old is None or old['start_ticks'] != row['start_ticks']:
            continue
        ticks = row['cpu_ticks'] - old['cpu_ticks']
        if ticks:
            active.append({**row, 'owned_measurement_tree': pid in owned,
                           'cpu_core_percent': ticks / os.sysconf('SC_CLK_TCK') / elapsed * 100})
    active.sort(key=lambda row: row['cpu_core_percent'], reverse=True)
    changes = [b - a for a, b in zip(before['cpu'], after['cpu'], strict=True)]
    total = sum(changes)
    return {'monotonic_ns': after['monotonic_ns'], 'interval_seconds': elapsed,
            'system_busy_percent': (total - changes[3] - changes[4]) / total * 100 if total else None,
            'system_iowait_percent': changes[4] / total * 100 if total else None,
            'observed_external_cpu_core_percent': sum(
                row['cpu_core_percent'] for row in active if not row['owned_measurement_tree']),
            'top_active_processes': active[:12], 'loadavg': after['loadavg'],
            'io_pressure': after['io_pressure'], 'memory_pressure': after['memory_pressure']}


def save(path, value):
    with path.open('x') as stream:
        json.dump(value, stream, indent=2)
        stream.write('\n')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    parser.add_argument('command', nargs=argparse.REMAINDER)
    args = parser.parse_args()
    os.chdir(ROOT)
    os.umask(0o077)
    output = args.output.absolute()
    assert output.parent == ROOT / '.local/p14-measurement-load' and not output.exists()
    command = args.command[1:] if args.command[:1] == ['--'] else args.command
    assert command
    output.mkdir(parents=True, mode=0o700)
    before = snapshot()
    forbidden = {'llama-server', 'cargo', 'rustc', 'steam', 'steamwebhelper', 'ninja',
                 'repository-owne', 'repository-cli', 'speed-owner', 'release-quality',
                 'ag1-process-hos'}
    competing = [row for row in before['processes'].values() if row['comm'] in forbidden]
    save(output / 'preflight.json', {
        'competing_processes': competing, 'command': command,
        'scope': 'Reject an already running model, compiler or Steam job. Retain remaining load for review; this check alone does not certify quiet conditions.',
        'observer': ref(Path(__file__).resolve()),
    })
    if competing:
        raise RuntimeError('Existing workload excludes measurement admission')
    stop = threading.Event()
    samples = []
    errors = []
    process = None

    def observe():
        prior = before
        try:
            with (output / 'samples.jsonl').open('x') as stream:
                while not stop.wait(1):
                    current = snapshot()
                    row = interval(prior, current, process.pid)
                    stream.write(json.dumps(row) + '\n')
                    stream.flush()
                    samples.append(row)
                    prior = current
        except Exception as error:
            errors.append(repr(error))

    with (output / 'command.log').open('x') as log:
        process = subprocess.Popen(command, stdout=log, stderr=subprocess.STDOUT)
        observer = threading.Thread(target=observe, daemon=True)
        observer.start()
        try:
            code = process.wait()
        finally:
            stop.set()
            observer.join(timeout=5)
            save(output / 'result.json', {
                'command': command, 'exit_code': process.poll(), 'pid': process.pid,
                'samples': len(samples), 'observer_errors': errors,
                'observer_thread_settled': not observer.is_alive(),
                'release_acceptance': False,
                'scope': 'All observed intervals retained, including noisy samples. External CPU estimate omits processes born and exited between samples; ancestry is observational and grants no process-control rights. Review actual measurement results separately.',
            })
    if errors or observer.is_alive():
        raise RuntimeError('Incomplete contention observation')
    raise SystemExit(code)


if __name__ == '__main__':
    main()
