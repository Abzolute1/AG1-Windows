#!/usr/bin/env python3
"""Freeze the pinned source archive and build a separate disposable reference."""
from pathlib import Path
import argparse
import os
import shutil
import subprocess
import tarfile

from common import ROOT, ORIGINAL, REFERENCE, EVIDENCE, PIN, GateError, run, sha256, verify_original, write_json, read_json, isolated_env


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--rebuild', action='store_true')
    args = parser.parse_args()
    fingerprints = verify_original()
    archive = ROOT / f'reference/ag1-{PIN}.tar'
    archive.parent.mkdir(exist_ok=True)
    archive_manifest = ROOT / 'reference/archive.json'
    if archive.exists():
        if not archive_manifest.exists() or sha256(archive) != read_json(archive_manifest)['sha256']:
            raise GateError('Existing reference archive is unverified; refusing to replace it')
    else:
        with archive.open('wb') as stream:
            subprocess.run(['git', '-C', str(ORIGINAL), 'archive', PIN], stdout=stream, check=True)
        archive.chmod(0o444)
        write_json(archive_manifest, {'commit': PIN, 'sha256': sha256(archive)})
    if not REFERENCE.exists():
        staging = ROOT / 'reference/ag1-executable.staging'
        if staging.exists():
            raise GateError('Interrupted reference staging exists; inspect before retry')
        staging.mkdir()
        with tarfile.open(archive) as bundle:
            bundle.extractall(staging, filter='data')
        # Copy, not hardlink/symlink, the known local dependency installation.
        source_dependencies = ROOT / 'research/scratch/ag1-reference/node_modules'
        if not source_dependencies.is_dir():
            raise GateError('Research dependencies missing; use a reviewed isolated install')
        subprocess.run(['cp', '-a', '--reflink=auto', str(source_dependencies), str(staging / 'node_modules')], check=True)
        staging.rename(REFERENCE)
    # Workspace symlinks in node_modules must resolve into this executable copy.
    for link in (REFERENCE / 'node_modules/@ag1').iterdir():
        if link.is_symlink() and not link.resolve().is_relative_to(REFERENCE):
            raise GateError(f'Dependency workspace link escapes reference copy: {link}')
    source_files = {}
    with tarfile.open(archive) as bundle:
        for member in bundle.getmembers():
            if member.isfile():
                path = REFERENCE / member.name
                original_bytes = bundle.extractfile(member).read()
                import hashlib
                digest = hashlib.sha256(original_bytes).hexdigest()
                if sha256(path) != digest:
                    raise GateError(f'Executable reference source modified: {member.name}')
                source_files[member.name] = digest
    write_json(ROOT / 'reference/source-files.json', source_files)
    commands = []
    if args.rebuild or not (EVIDENCE / 'bootstrap.json').exists():
        commands.append(run(['npm', 'run', 'build', '--workspace=packages/tui',
                             '--workspace=packages/ai', '--workspace=packages/agent',
                             '--workspace=packages/coding-agent', '--workspace=packages/godot-agent-runtime'],
                            cwd=REFERENCE, log=EVIDENCE / 'build.log',
                            env=isolated_env(ROOT / '.local/build')))
    tools = {name: subprocess.check_output(command, text=True).strip() for name, command in
             [('node', ['node', '--version']), ('npm', ['npm', '--version']), ('python', ['python3', '--version'])]}
    dependencies = REFERENCE / 'node_modules/.package-lock.json'
    report = {'status': 'passed', 'input_fingerprints': fingerprints,
              'archive': str(archive.relative_to(ROOT)), 'archive_sha256': sha256(archive),
              'source_manifest_sha256': sha256(ROOT / 'reference/source-files.json'),
              'source_files': len(source_files), 'lockfile_sha256': sha256(REFERENCE / 'package-lock.json'),
              'installed_lockfile_sha256': sha256(dependencies) if dependencies.exists() else None,
              'toolchain': tools, 'commands': commands,
              'reference': str(REFERENCE.relative_to(ROOT)),
              'dependency_origin': 'Copied research installation, no links to original repository; no fresh registry resolution.'}
    write_json(EVIDENCE / 'bootstrap.json', report)
    print(f'Reference ready: {len(source_files)} pinned files; separate rebuilt execution tree.')


if __name__ == '__main__':
    main()
