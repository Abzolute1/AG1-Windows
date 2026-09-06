#!/usr/bin/env python3
"""Record licenses from exact locked crate sources, including build and target dependencies."""
import json
from pathlib import Path
import shutil
import subprocess
import urllib.request

from common import ROOT, REFERENCE, GateError, read_json, sha256, write_json


def main():
    metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--manifest-path', str(ROOT/'product/Cargo.toml'), '--locked', '--offline', '--format-version', '1']))
    destination = ROOT/'product/licenses'
    records = []
    for package in metadata['packages']:
        if not package['source']:
            continue
        directory = Path(package['manifest_path']).parent
        files = [f for f in directory.iterdir() if f.is_file() and (f.name.lower().startswith(('license', 'licence', 'copying', 'notice', 'copyright')) or f.name.lower().endswith(('-license', '-licence')) or f.name.lower() == 'unlicense')]
        for folder in directory.iterdir():
            if folder.is_dir() and folder.name.lower() in ['licenses', 'license']:
                files.extend(f for f in folder.rglob('*') if f.is_file())
        if not files and (directory/'AUTHORS').is_file() and 'AUTHORS-MIT:' in (directory/'AUTHORS').read_text():
            files.append(directory/'AUTHORS')
        target = destination/f"{package['name']}-{package['version']}"
        target.mkdir(parents=True, exist_ok=True)
        copied = []
        for file in files:
            output = target/file.name
            shutil.copyfile(file, output)
            copied.append({'path': str(output.relative_to(ROOT)), 'sha256': sha256(output), 'source': str(file)})
        if not copied and package['name'] in ['ts-rs', 'ts-rs-macros']:
            commit = read_json(directory/'.cargo_vcs_info.json')['git']['sha1']
            url = f'https://raw.githubusercontent.com/Aleph-Alpha/ts-rs/{commit}/LICENSE'
            output = target/'LICENSE'
            if not output.exists():
                with urllib.request.urlopen(url, timeout=30) as response:
                    output.write_bytes(response.read())
            if 'MIT License' not in output.read_text() or 'Aleph Alpha' not in output.read_text():
                raise GateError('Unexpected ts-rs license text')
            copied.append({'path': str(output.relative_to(ROOT)), 'sha256': sha256(output), 'source': url, 'reason': 'Published crate omits repository license; pinned VCS commit used.'})
        if not copied:
            fallback = destination/'crate-license-fallbacks.json'
            key = f"{package['name']}@{package['version']}"
            copied = read_json(fallback).get(key, []) if fallback.exists() else []
            for row in copied:
                if sha256(ROOT/row['path']) != row['sha256']: raise GateError('License fallback drift')
        if not copied:
            raise GateError(f"No license text for {package['name']}")
        records.append({'name': package['name'], 'version': package['version'], 'license': package['license'], 'source': package['source'], 'files': copied})
    for name, version in [('typescript', '5.9.3'), ('ajv', '8.18.0')]:
        directory = REFERENCE/'node_modules'/name
        if read_json(directory/'package.json')['version'] != version:
            raise GateError('Reference checker dependency version changed')
        target = destination/f'{name}-{version}'; target.mkdir(exist_ok=True)
        copied = []
        for file in directory.iterdir():
            if file.is_file() and file.name.lower().startswith(('license', 'notice', 'thirdpartynotice')):
                output = target/file.name; shutil.copyfile(file, output)
                copied.append({'path': str(output.relative_to(ROOT)), 'sha256': sha256(output), 'source': str(file)})
        if not copied:
            raise GateError(f'Missing checker license: {name}')
        records.append({'name': name, 'version': version, 'license': read_json(directory/'package.json')['license'], 'scope': 'development checker from immutable reference; not frontend dependencies', 'files': copied})
    write_json(destination/'manifest.json', {'cargo_lock_sha256': sha256(ROOT/'product/Cargo.lock'), 'packages': records})
    print(f'Recorded {len(records)} exact dependency license sets')


if __name__ == '__main__':
    main()
