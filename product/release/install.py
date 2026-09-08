#!/usr/bin/env python3
"""Local review installer. Versioned payloads; explicit SHA-256; no state migration.

Python is a packaging/review prerequisite, not a running application dependency.
An interrupted/unknown operation is retained and requires inspection before retry.
"""
import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import tarfile
import uuid


PROTECTED = [Path(p) for p in (
    '/home/alex/Projects/Cruzible', '/home/alex/Projects/AG1/vendor',
    '/home/alex/Projects/AG1/reference', '/home/alex/Projects/AG1/product',
    '/home/alex/Projects/AG1/.local/p13-production-20260907',
    '/home/alex/Projects/AG1/.local/p13-runtime-capture-20260907',
    '/home/alex/Projects/llama-cpp-turboquant', '/home/alex/Projects/llama-cpp-qwen4exp',
    '/home/alex/Projects/llama-cpp-qwen4exp-mtp-b10715', '/home/alex/.ag1',
    '/home/alex/.local/state/ag1')]


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def sync_directory(path):
    fd = os.open(path, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def create_json(path, value):
    with path.open('x') as stream:
        json.dump(value, stream, indent=2)
        stream.flush()
        os.fsync(stream.fileno())
    sync_directory(path.parent)


def install(source, expected, destination, fault=lambda stage: None):
    source, destination = Path(source), Path(os.path.abspath(destination))
    if any(p.is_symlink() for p in [destination, *destination.parents]):
        raise ValueError('Installation path cannot traverse a symlink')
    if any(destination == p or destination.is_relative_to(p) or p.is_relative_to(destination) for p in PROTECTED):
        raise ValueError('Installation cannot write original source, state, archives or runtimes')
    if source.is_symlink() or digest(source) != expected:
        raise ValueError('Candidate archive identity mismatch')
    if not destination.exists():
        destination.mkdir(mode=0o700, parents=False)
        create_json(destination/'installation.json', {'version':1,'id':uuid.uuid4().hex})
    if (destination/'installation.json').is_symlink() or not (destination/'installation.json').is_file():
        raise ValueError('Existing directory is not an AG1 review installation')
    marker = json.loads((destination/'installation.json').read_text())
    if marker.get('version') != 1 or not marker.get('id'):
        raise ValueError('Unknown installation identity')
    for name in ('install.lock','pending.json','releases','receipts','current.next'):
        if (destination/name).is_symlink():
            raise ValueError('Installation metadata cannot be redirected')
    with (destination/'install.lock').open('a') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        if (destination/'pending.json').exists():
            raise ValueError('Unknown pending operation; inspect retained stage/receipt before retry')
        operation = uuid.uuid4().hex
        releases, receipts = destination/'releases', destination/'receipts'
        releases.mkdir(exist_ok=True)
        receipts.mkdir(exist_ok=True)
        stage = releases/(operation+'.staging')
        final = releases/operation
        stage.mkdir()
        create_json(destination/'pending.json', {'operation':operation,'archive_sha256':expected,'stage':str(stage)})
        fault('pending')
        # Bind extraction to the actual opened inode, including a path replacement
        # between the preliminary check and this open. Recheck that inode afterward.
        with source.open('rb') as raw:
            raw.seek(0)
            if hashlib.file_digest(raw, 'sha256').hexdigest() != expected:
                raise ValueError('Candidate archive identity changed before extraction')
            raw.seek(0)
            with tarfile.open(fileobj=raw, mode='r:gz') as tar:
                members = tar.getmembers()
                names = [m.name for m in members]
                if len(names) != len(set(names)) or len(names) > 10000 or sum(m.size for m in members) > 2*1024**3:
                    raise ValueError('Invalid package inventory or bound')
                for member in members:
                    relative = Path(member.name)
                    if not member.isfile() or relative.is_absolute() or '..' in relative.parts or not relative.parts or member.mode & 0o7000:
                        raise ValueError('Package contains an unsafe path, link or special file')
                    target = stage/relative
                    target.parent.mkdir(parents=True, exist_ok=True)
                    content = tar.extractfile(member)
                    with target.open('xb') as out:
                        while chunk := content.read(1024*1024):
                            out.write(chunk)
                        out.flush()
                        os.fsync(out.fileno())
                    target.chmod(member.mode & 0o755)
            raw.seek(0)
            if hashlib.file_digest(raw, 'sha256').hexdigest() != expected:
                raise ValueError('Candidate archive inode changed during extraction')
        manifest = json.loads((stage/'package-manifest.json').read_text())
        actual = {p.relative_to(stage).as_posix() for p in stage.rglob('*') if p.is_file()}
        files = manifest.get('files', [])
        if not files or manifest.get('version') != 1 or manifest.get('platform') != 'linux-x86_64' or len(files) != len({r['path'] for r in files}) or actual != {r['path'] for r in files} | {'package-manifest.json'}:
            raise ValueError('Candidate manifest does not cover the exact payload')
        for row in files:
            path = stage/row['path']
            if path.stat().st_size != row['bytes'] or digest(path) != row['sha256'] or path.stat().st_mode & 0o777 != row['mode']:
                raise ValueError('Candidate payload differs from manifest')
        if digest(source) != expected:
            raise ValueError('Candidate archive changed during installation')
        for folder in sorted((p for p in stage.rglob('*') if p.is_dir()), key=lambda p:len(p.parts), reverse=True):
            sync_directory(folder)
        sync_directory(stage)
        fault('verified')
        stage.rename(final)
        sync_directory(releases)
        create_json(receipts/(operation+'.json'), {'operation':operation,'archive_sha256':expected,'release':str(final),'previous':os.readlink(destination/'current') if (destination/'current').is_symlink() else None})
        fault('published')
        os.symlink('releases/'+operation, destination/'current.next')
        os.replace(destination/'current.next', destination/'current')
        sync_directory(destination)
        fault('activated')
        (destination/'pending.json').unlink()
        sync_directory(destination)
        return final


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('archive', type=Path)
    parser.add_argument('sha256', help='SHA-256 from the reviewed P14 package report')
    parser.add_argument('destination', type=Path, help='New directory, or an existing review installation')
    args = parser.parse_args()
    print(install(args.archive,args.sha256,args.destination))
