"""Original repair criteria through full, integrity-checked native detail pages."""
import base64
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time
import zlib
from faults import retain,digest


def query(client, kind, action, args):
    result = client.execute({'kind': kind, 'action': action, 'args': args}, seconds=60)
    return result['acknowledgement']['data']


def page(client, kind, action, args):
    offset = '0'
    expected = None
    result = bytearray()
    while offset is not None:
        parameters = {**args, 'offset': offset}
        if expected is not None and action == 'detail_page':
            parameters['sha256'] = expected
        value = query(client, kind, action, parameters)
        if expected is None:
            expected = value['sha256']
        if value['sha256'] != expected:
            raise RuntimeError('Native detail changed during pagination')
        result.extend(base64.b64decode(value['base64'], validate=True))
        if len(result) > 16 * 1024 * 1024:
            raise RuntimeError('Unbounded native detail cannot become benchmark evidence')
        next_offset = value['next']
        if next_offset is not None and int(next_offset) <= int(offset):
            raise RuntimeError('Native detail pagination failed to advance')
        offset = next_offset
    if hashlib.sha256(result).hexdigest() != expected:
        raise RuntimeError('Native detail bytes differ from their declared hash')
    return json.loads(result)


def history(client, offset=0):
    result = []
    while offset is not None:
        value = query(client, 'session', 'history', {'offset': offset})
        result.extend(value['entries'])
        if len(result) > 100000:
            raise RuntimeError('Unbounded history evidence')
        offset = value['next']
    return result


def records_since(client, offset):
    entries = history(client, offset)
    return [page(client, 'session', 'entry', {'id': row['id']}) for row in entries]


def git(workspace, *args):
    result = subprocess.run(['/usr/bin/git', *args], cwd=workspace, capture_output=True,
                            text=True, check=True, timeout=60)
    return result.stdout


def introduce(workspace, spec, serial):
    if git(workspace, 'status', '--porcelain').strip():
        raise RuntimeError('Workload cannot replace uncommitted changes')
    (workspace / spec['sourcePath']).write_text(spec['brokenSource'])
    git(workspace, 'add', spec['sourcePath'])
    git(workspace, 'commit', '-q', '-m', f'P14 regression {serial}')


def verify(client, workspace, state, specs, spec, before, output, node, context_override=None):
    output = Path(output)
    records = records_since(client, before)
    retain(output / 'records.json', records)
    recall = any(r.get('type') == 'message' and r.get('message', {}).get('role') == 'toolResult'
                 and r['message'].get('toolName') == 'artifact'
                 and any(b.get('type') == 'text' and spec['currentContract'] in b.get('text', '')
                         for b in r['message'].get('content', [])) for r in records)
    context = context_override if context_override is not None else page(client, 'runtime', 'detail_page', {'section': 'context'})
    retain(output / 'context.json', context)
    within = isinstance(context, dict) and context.get('stats', {}).get('overTargetTokens') == 0
    if digest(node['path']) != node['sha256']:
        raise RuntimeError('Bound verifier interpreter changed')
    checked = subprocess.run([node['path'], spec['verification']], cwd=workspace,
                              capture_output=True, timeout=60)
    (output / 'verifier.log').write_bytes(checked.stdout + checked.stderr)
    changed = git(workspace, 'status', '--porcelain')
    scoped = all(line[3:] == spec['sourcePath'] for line in changed.splitlines())
    unchanged = all((workspace / s['verification']).read_text() == s['verificationSource'] for s in specs)
    diff = git(workspace, 'diff', '--no-ext-diff', '--', spec['sourcePath'])
    (output / 'diff.patch').write_text(diff)
    result = {'id': spec['id'], 'required_recall': recall, 'external_verifier_exit_code': checked.returncode,
              'verifiers_unchanged': unchanged, 'mutation_scope_passed': scoped,
              'active_context_within_target': within, 'changed_files': changed,
              'criteria': 'Exact current contract in an actual artifact tool result; unchanged original external source verifier; allowed mutation path only; full native compilation stats'}
    result['status'] = 'passed' if recall and checked.returncode == 0 and unchanged and scoped and within else 'failed'
    retain(output / 'verification.json', result)
    if result['status'] != 'passed':
        raise RuntimeError(f'Original workload verification failed: {result}')
    git(workspace, 'add', spec['sourcePath'])
    git(workspace, 'commit', '-q', '-m', 'P14 verified repair')
    return result


def artifacts_intact(state, seed):
    for artifact in seed['artifacts']:
        expected = artifact['sha256']
        if logical_blob_digest(state / 'store/blobs', expected) != expected:
            raise RuntimeError('Original externalized artifact changed')
    return True


def logical_blob_digest(directory, expected):
    """Hash the complete bounded logical bytes of a v1/raw or v2/custody blob."""
    if len(expected) != 64 or any(c not in '0123456789abcdef' for c in expected):
        raise RuntimeError('Invalid logical blob digest')
    raw = directory / expected
    if os.path.lexists(raw):
        if raw.is_symlink() or not raw.is_file():
            raise RuntimeError('Invalid raw blob')
        return digest(raw)
    path = directory / (expected + '.z1')
    if path.is_symlink() or not path.is_file() or path.stat().st_size > 256 * 1024 * 1024:
        raise RuntimeError('Invalid compressed blob')
    hasher = hashlib.sha256()
    total = 0
    with path.open('rb') as source:
        header = source.read(16)
        if len(header) != 16 or header[:8] != b'AG1ZLIB1':
            raise RuntimeError('Unsupported compressed blob')
        length = int.from_bytes(header[8:], 'little')
        if length > 256 * 1024 * 1024:
            raise RuntimeError('Oversized compressed blob')
        decoder = zlib.decompressobj()
        while block := source.read(65536):
            while block:
                decoded = decoder.decompress(block, min(65536, length - total + 1))
                total += len(decoded)
                if total > length:
                    raise RuntimeError('Compressed blob length exceeded')
                hasher.update(decoded)
                block = decoder.unconsumed_tail
                if decoder.eof:
                    if decoder.unused_data or source.read(1) or total != length:
                        raise RuntimeError('Compressed blob length mismatch or trailing bytes')
                    return hasher.hexdigest()
        raise RuntimeError('Truncated compressed blob')
