#!/usr/bin/env python3
"""Copy or verify exact offline extension-loader dependencies; no package scripts."""
import hashlib
import json
from pathlib import Path
import shutil

ROOT = Path(__file__).resolve().parents[1]
PACKAGES = {'@mariozechner/jiti': '2.6.5', '@sinclair/typebox': '0.34.49', 'std-env': '3.10.0', 'yoctocolors': '2.1.2'}
manifest = {'version': 1, 'source_commit': 'e30f0b4b238ee5618928bcd17b4571eb046e7038', 'packages': []}
for name, version in PACKAGES.items():
    source = ROOT / 'reference/ag1-executable/node_modules' / name
    destination = ROOT / 'product/compat/extensions/vendor/node_modules' / name
    package = json.loads((source / 'package.json').read_text())
    assert package['name'] == name and package['version'] == version
    files = []
    for path in sorted(source.rglob('*'), key=lambda p: p.relative_to(source).as_posix()):
        assert not path.is_symlink(), path
        if not path.is_file():
            continue
        relative = path.relative_to(source)
        data = path.read_bytes()
        assert len(data) <= 4 * 1024 * 1024
        target = destination / relative
        if target.exists():
            assert target.read_bytes() == data, target
        else:
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(path, target)
        files.append({'path': relative.as_posix(), 'bytes': len(data), 'sha256': hashlib.sha256(data).hexdigest()})
    assert len(files) <= 2000 and sum(f['bytes'] for f in files) <= 8 * 1024 * 1024
    assert sorted(p.relative_to(destination).as_posix() for p in destination.rglob('*') if p.is_file()) == [f['path'] for f in files]
    license_files = [f['path'] for f in files if Path(f['path']).name.lower() in ['license', 'licence']]
    assert license_files
    manifest['packages'].append({'name': name, 'version': version, 'license': package['license'], 'notices': license_files, 'files': files})
target = ROOT / 'product/compat/extensions/dependencies.json'
data = json.dumps(manifest, indent=2) + '\n'
if target.exists():
    assert target.read_text() == data
else:
    target.write_text(data)
print(json.dumps({'status': 'passed', 'packages': len(PACKAGES), 'files': sum(len(p['files']) for p in manifest['packages']), 'network_actions': 0, 'package_scripts': 0}))
