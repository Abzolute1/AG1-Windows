"""Observe explicitly launched P14 process trees and the native glibc allocator."""
import json
import os
from pathlib import Path
import threading
import time
import xml.etree.ElementTree as ET


def identity(pid):
    try:
        base = Path('/proc') / str(pid)
        fields = (base / 'stat').read_text().rsplit(')', 1)[1].split()
        exe = (base / 'exe').stat()
        return {'pid': pid, 'ppid': int(fields[1]), 'start_ticks': int(fields[19]),
                'state': fields[0], 'executable_device': exe.st_dev,
                'executable_inode': exe.st_ino, 'executable': str((base / 'exe').resolve())}
    except (FileNotFoundError, ProcessLookupError, PermissionError):
        return None


def same_process(a, b):
    return a is not None and b is not None and all(a[k] == b[k] for k in
        ('pid', 'start_ticks', 'executable_device', 'executable_inode'))


def descendants(root):
    current = identity(root['pid'])
    if not same_process(root, current):
        return []
    rows = {int(p.name): identity(int(p.name)) for p in Path('/proc').iterdir() if p.name.isdigit()}
    chosen = {root['pid']}
    while True:
        next_set = chosen | {pid for pid, row in rows.items() if row and row['ppid'] in chosen}
        if next_set == chosen:
            break
        chosen = next_set
    result = []
    for pid in sorted(chosen):
        row = rows.get(pid)
        if not row or row['state'] == 'Z':
            continue
        try:
            lines = Path('/proc', str(pid), 'smaps_rollup').read_text().splitlines()
            row['pss_bytes'] = next(int(line.split()[1]) * 1024 for line in lines if line.startswith('Pss:'))
        except (FileNotFoundError, ProcessLookupError, PermissionError):
            continue
        if same_process(row, identity(pid)):
            result.append(row)
    return result


def heap_snapshot(path):
    with path.open('rb') as stream:
        stream.seek(max(0, path.stat().st_size - 4 * 1024 * 1024))
        lines = stream.read().splitlines()
    # The writer fsyncs every completed line; an in-progress trailing line is ignored.
    for line in reversed(lines):
        try:
            raw = json.loads(line)
        except (ValueError, UnicodeDecodeError):
            continue
        if raw['malloc_info_result'] != 0:
            raise RuntimeError('Native malloc_info sampling failed')
        xml = ET.fromstring(raw['xml'])
        totals = {r.attrib['type']: int(r.attrib['size']) for r in xml.findall('total')}
        current = int(xml.find("system[@type='current']").attrib['size'])
        reserved = current + totals['mmap']
        estimate = reserved - totals['rest'] - totals.get('fast', 0)
        if estimate < 0:
            raise RuntimeError('Invalid allocator accounting')
        return {'monotonic_ns': raw['monotonic_ns'], 'pid': raw['pid'],
                'allocator_reserved_bytes': reserved, 'heap_bytes': estimate,
                'arenas': len(xml.findall('heap')), 'raw_file': str(path)}
    raise RuntimeError('No complete native allocator observation')


def directory_bytes(path):
    total = 0
    for file in path.rglob('*'):
        try:
            if file.is_file() and not file.is_symlink():
                total += file.stat().st_size
        except FileNotFoundError:
            pass
    return total


class Sampler:
    def __init__(self, output, state):
        self.output = Path(output)
        self.state = Path(state)
        self.lock = threading.Lock()
        self.sample_lock = threading.Lock()
        self.active = None
        self.samples = []
        self.known_children = {}
        self.error = None
        self.stopped = threading.Event()
        self.thread = threading.Thread(target=self._run, daemon=True)
        self.log = self.output.open('x')

    def bind(self, client):
        current = identity(client.process.pid)
        if current is None:
            raise RuntimeError('Cannot observe launched native client')
        with self.lock:
            self.active = (current, client.output / f'heap.{client.process.pid}.jsonl')

    def unbind(self):
        with self.lock:
            self.active = None

    def start(self):
        self.thread.start()

    def sample(self):
        with self.sample_lock:
            self._sample()

    def _sample(self):
        now = time.monotonic_ns()
        with self.lock:
            active = self.active
        rows = descendants(active[0]) if active else []
        host_alive = active and any(r['pid'] == active[0]['pid'] for r in rows)
        if active:
            with self.lock:
                self.known_children.update({r['pid']:dict(r) for r in rows if r['pid']!=active[0]['pid']})
        heap = heap_snapshot(active[1]) if host_alive else None
        if heap and (now - heap['monotonic_ns']) / 1e9 > 120:
            raise RuntimeError('Allocator observation is stale')
        row = {'monotonic_ns': now, 'heap_bytes': heap['heap_bytes'] if heap else 0,
               'pss_bytes': sum(r['pss_bytes'] for r in rows),
               'persistent_evidence_bytes': directory_bytes(self.state),
               'owned_children': max(0, len(rows) - 1), 'processes': rows,
               'host_present': bool(host_alive), 'heap_observation': heap,
               'heap_scope': 'native host glibc arenas/mmap, allocator in-use-or-retained estimate; model/helper/webview heaps excluded, included in tree PSS'}
        self.samples.append(row)
        self.log.write(json.dumps(row) + '\n')
        self.log.flush()
        os.fsync(self.log.fileno())

    def _run(self):
        try:
            while not self.stopped.is_set():
                self.sample()
                self.stopped.wait(60)
        except Exception as error:
            self.error = repr(error)

    def close(self):
        self.stopped.set()
        self.thread.join(timeout=30)
        if self.thread.is_alive():
            raise RuntimeError('Memory sampler did not stop')
        self.log.close()
        if self.error:
            raise RuntimeError(self.error)
