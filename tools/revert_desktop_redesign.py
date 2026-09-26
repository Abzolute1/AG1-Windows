#!/usr/bin/env python3
"""Review or restore the exact pre-redesign source without overwriting later edits."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import tarfile
import tempfile


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest() if path.is_file() else None


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--baseline', type=Path, default=Path('.local/p14-ui-redesign-baseline-1'))
    parser.add_argument('--apply', action='store_true', help='Restore after validating every current file')
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    baseline = (root / args.baseline).resolve()
    changes = json.loads((baseline / 'changes.json').read_text())
    conflicts = []
    for name, expected in changes.items():
        path = root / name
        if not path.resolve().is_relative_to(root) or path.is_symlink() or any(p.is_symlink() for p in path.parents if p != root):
            raise SystemExit(f'Unsafe restore path: {name}')
        if digest(path) != expected['after']:
            conflicts.append(name)
    if conflicts:
        raise SystemExit('Later edits detected; no files changed:\n' + '\n'.join(conflicts))
    if not args.apply:
        print(f'Ready to restore {len(changes)} source files. No files changed. Use --apply to restore.')
        return
    with tarfile.open(baseline / 'source.tar.gz') as archive:
        # Validate all original contents before changing any source file.
        originals = {}
        for name, expected in changes.items():
            if expected['before'] is not None:
                info = archive.getmember(name)
                data = archive.extractfile(info).read()
                if hashlib.sha256(data).hexdigest() != expected['before']:
                    raise SystemExit(f'Backup content mismatch: {name}')
                originals[name] = (data, info.mode)
        for name in changes:
            path = root / name
            if name not in originals:
                path.unlink(missing_ok=True)
                continue
            data, mode = originals[name]
            path.parent.mkdir(parents=True, exist_ok=True)
            descriptor, temporary = tempfile.mkstemp(dir=path.parent)
            try:
                with os.fdopen(descriptor, 'wb') as stream:
                    stream.write(data)
                    stream.flush()
                    os.fsync(stream.fileno())
                os.chmod(temporary, mode)
                os.replace(temporary, path)
            finally:
                Path(temporary).unlink(missing_ok=True)
    print('Pre-redesign source restored. Chats, settings and installed releases were not modified.')


if __name__ == '__main__':
    main()
