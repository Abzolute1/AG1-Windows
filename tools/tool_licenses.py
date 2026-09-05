"""Offline validation of P09 native ports and the unchanged optional loader bundle."""
import json
import re
from pathlib import Path
from common import ROOT, PIN, GateError, read_json, sha256


def files(rows):
    if not rows:
        raise GateError('Missing dependency notices or source material')
    for row in rows:
        relative = row['path']
        path = (ROOT / relative).resolve()
        if not path.is_relative_to(ROOT) or not path.is_file() or sha256(path) != row['sha256']:
            raise GateError(f'Stale or escaping license provenance: {relative}')


def bundle_closure(lock, code):
    roots = set(re.findall(r'\./node_modules/((?:@[^/" ]+/)?[^/" ]+)', code)) | {'acorn'}
    seen = set()
    def resolve(base, name):
        while True:
            key = (base + '/' if base else '') + 'node_modules/' + name
            if key in lock:
                return key
            if not base:
                raise GateError(f'Unresolved bundled dependency: {name}')
            base = base.rsplit('/node_modules/', 1)[0] if '/node_modules/' in base else ''
    def visit(key):
        if key in seen:
            return
        seen.add(key)
        for name in lock[key].get('dependencies', {}):
            visit(resolve(key, name))
    for name in roots:
        visit(resolve('', name))
    return seen


def verify():
    cargo = read_json(ROOT / 'product/licenses/manifest.json')
    if cargo.get('cargo_lock_sha256') != sha256(ROOT / 'product/Cargo.lock'):
        raise GateError('Stale native dependency notices')
    for row in cargo['packages']:
        files(row['files'])
    ports = read_json(ROOT / 'fixtures/tools/port-provenance.json')
    if ports.get('source_commit') != PIN or {p['name'] for p in ports['ports']} != {'diff', 'ansi-regex', 'partial-json'}:
        raise GateError('Missing native port provenance')
    for row in ports['ports']:
        files(row['source_files']); files(row['notices'])
    bundle = read_json(ROOT / 'product/compat/extensions/bundle-notices.json')
    if bundle.get('source_commit') != 'd45d2c2175f04d760bee36a12dfdb24cd8a18eb3':
        raise GateError('Changed loader source commit')
    files(bundle['source_files']); files(bundle['package']['files'])
    lock = read_json(ROOT / 'fixtures/tools/jiti-source/package-lock.json')['packages']
    base = ROOT / 'product/compat/extensions/vendor/node_modules/@mariozechner/jiti'
    code = '\n'.join(p.read_text() for p in (base / 'dist').glob('*.cjs'))
    expected = bundle_closure(lock, code)
    components = bundle['components']
    if len(expected) != 67 or len(components) != len(expected) or {r['lock_path'] for r in components} != expected:
        raise GateError('Incomplete bundled loader dependency closure')
    for row in components:
        source = lock[row['lock_path']]
        if (row['version'] != source['version'] or row['integrity'] != source['integrity']
            or row['archive_url'] != source['resolved'] or row['license'] != source['license']):
            raise GateError('Bundle dependency differs from the published source lock')
        files(row['files'])
    exceptions = bundle.get('exceptions', [])
    if len(exceptions) != 1 or exceptions[0].get('name') != 'babel-plugin-parameter-decorator' or not exceptions[0].get('reason'):
        raise GateError('Missing exact declared-license fallback explanation')
    metadata = read_json(ROOT / 'product/compat/extensions/notices/babel-plugin-parameter-decorator-1.0.16/package.json')
    if metadata.get('version') != '1.0.16' or metadata.get('license') != 'MIT' or metadata.get('author') != 'Warner':
        raise GateError('Changed published fallback license declaration')
    return {'status': 'passed', 'native_dependency_sets': len(cargo['packages']),
            'native_ports': len(ports['ports']), 'loader_bundle_components': len(components),
            'bundle_notice_files': sum(len(r['files']) for r in components),
            'declared_license_fallbacks': exceptions, 'network_actions': 0}


if __name__ == '__main__':
    print(json.dumps(verify()))
