"""Fault injection limited to the disposable CLI launch and its authenticated tree."""
import hashlib
import json
import os
from pathlib import Path
import select
import signal
import time
from resources import descendants, identity, same_process


def digest(path):
    h = hashlib.sha256()
    with Path(path).open('rb') as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b''):
            h.update(chunk)
    return h.hexdigest()


def retain(path, value):
    with Path(path).open('x') as stream:
        json.dump(value, stream, indent=2)
        stream.write('\n')
        stream.flush()
        os.fsync(stream.fileno())


def exited(fd, seconds):
    poller = select.poll()
    poller.register(fd, select.POLLIN)
    return bool(poller.poll(round(seconds * 1000)))


def crash_model(client, expected_plan, bound_helper, output):
    """No endpoint/PID adoption: require this launched host's pinned helper child."""
    root = identity(client.process.pid)
    if not root or client.process.poll() is not None:
        raise RuntimeError('Disposable native owner is not running')
    helper_path = Path(bound_helper['path']).resolve()
    if digest(helper_path) != bound_helper['sha256']:
        raise RuntimeError('Bound helper changed')
    helpers = [r for r in descendants(root) if r['ppid'] == root['pid']
               and r['executable'] == str(helper_path)]
    command = expected_plan['command']
    candidates = []
    for helper in helpers:
        for child in descendants(helper):
            if child['ppid'] != helper['pid'] or child['executable'] != command:
                continue
            argv = Path('/proc', str(child['pid']), 'cmdline').read_bytes().split(b'\0')
            if argv[-1:] == [b'']:
                argv.pop()
            if argv != [os.fsencode(command), *[os.fsencode(a) for a in expected_plan['args']]]:
                raise RuntimeError('Owned model launch arguments differ from preserved plan')
            candidates.append((helper, child))
    if len(candidates) != 1:
        raise RuntimeError('Exactly one preserved model owned by this native launch is required')
    helper, target = candidates[0]
    fd = os.pidfd_open(target['pid'])
    try:
        root_now, helper_now, target_now = (identity(v['pid']) for v in (root, helper, target))
        if (not same_process(root, root_now) or not same_process(helper, helper_now)
                or not same_process(target, target_now) or helper_now['ppid'] != root['pid']
                or target_now['ppid'] != helper['pid'] or client.process.poll() is not None):
            raise RuntimeError('Owned process lineage changed before injection')
        receipt = {'kind': 'model-crash', 'started_monotonic_ns': time.monotonic_ns(),
                   'host': root, 'helper': helper, 'target': target,
                   'binding': 'direct child of pinned supervisor, direct child of this Popen native host; exact preserved command/arguments; pidfd held and all identities revalidated',
                   'signal': 'SIGKILL'}
        retain(Path(output) / 'model-crash-armed.json', receipt)
        signal.pidfd_send_signal(fd, signal.SIGKILL)
        if not exited(fd, 30):
            raise RuntimeError('Injected model did not exit')
        receipt.update(target_exited=True, finished_monotonic_ns=time.monotonic_ns())
        retain(Path(output) / 'model-crash-injected.json', receipt)
        return receipt
    finally:
        os.close(fd)


def crash_idle_host(client, output):
    if client.view['snapshot']['status'] != 'idle' or client.view['snapshot']['pending']:
        raise RuntimeError('This declared host fault is at a verified idle checkpoint')
    root = identity(client.process.pid)
    if not root or client.process.poll() is not None:
        raise RuntimeError('Disposable native owner is not running')
    held = []
    try:
        for row in descendants(root):
            if row['pid'] == root['pid']:
                continue
            try:
                fd = os.pidfd_open(row['pid'])
            except ProcessLookupError:
                continue
            if not same_process(row, identity(row['pid'])):
                os.close(fd)
                raise RuntimeError('Child identity changed during fault arming')
            held.append((row, fd))
        receipt = {'kind': 'host-crash', 'started_monotonic_ns': time.monotonic_ns(),
                   'scope': 'idle checkpoint after a verified repair and completed command receipt',
                   'host': root, 'owned_children': [r for r, _ in held], 'signal': 'SIGKILL'}
        retain(Path(output) / 'host-crash-armed.json', receipt)
        signal.pidfd_send_signal(client.pidfd, signal.SIGKILL)
        if not exited(client.pidfd, 30):
            raise RuntimeError('Injected host did not exit')
        client.process.wait(timeout=10)
        deadline = time.monotonic() + 30
        for row, fd in held:
            if not exited(fd, max(0, deadline - time.monotonic())):
                raise RuntimeError(f'Owned child failed to settle after host crash: {row["pid"]}')
        receipt.update(host_exit_code=client.process.returncode,
                       children_settled_without_external_signals=True,
                       finished_monotonic_ns=time.monotonic_ns())
        retain(Path(output) / 'host-crash-injected.json', receipt)
        return receipt
    finally:
        for _, fd in held:
            os.close(fd)
