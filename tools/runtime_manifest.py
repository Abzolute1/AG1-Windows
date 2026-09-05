#!/usr/bin/env python3
"""Read-only runtime identity capture. Never executes a model binary or HTTP call."""
from __future__ import annotations

import argparse
import hashlib
import os
from pathlib import Path
import re
import subprocess
import time

from common import ROOT, EVIDENCE, GateError, read_json, write_json, sha256


def signature(path: Path):
    stat = path.stat()
    return [stat.st_dev, stat.st_ino, stat.st_size, stat.st_mtime_ns, stat.st_ctime_ns]


def stable_hash(path: Path, cache: dict, *, force=False):
    resolved = path.resolve(strict=True)
    before = signature(resolved)
    prior = cache.get(str(resolved))
    if not force and prior and prior['signature'] == before:
        return prior['sha256'], True
    digest = sha256(resolved)
    if signature(resolved) != before or path.resolve(strict=True) != resolved:
        raise GateError(f'File changed while hashing: {path}')
    cache[str(resolved)] = {'signature': before, 'sha256': digest}
    return digest, False


def elf_dynamic(path: Path):
    result = subprocess.run(['readelf', '-d', str(path)], capture_output=True, text=True, check=True)
    needed = re.findall(r'\(NEEDED\).*?\[(.*?)\]', result.stdout)
    paths = re.findall(r'\((?:RUNPATH|RPATH)\).*?\[(.*?)\]', result.stdout)
    return needed, [part.replace('${ORIGIN}', str(path.parent)).replace('$ORIGIN', str(path.parent))
                    for item in paths for part in item.split(':') if part]


def loader_cache():
    result = subprocess.run(['ldconfig', '-p'], capture_output=True, text=True, check=True)
    cache = {}
    for name, path in re.findall(r'^\s*(\S+)\s+\([^\n]*\)\s+=>\s+(\S+)', result.stdout, re.M):
        cache.setdefault(name, []).append(Path(path))
    return cache


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--force-hash', action='store_true', help='Ignore stat-identity hash cache')
    args = parser.parse_args()
    start = time.monotonic()
    lock = read_json(ROOT / 'planning/evidence/llama-runtime-lock.json')
    profile_path = Path(lock['profile_file'])
    if sha256(profile_path) != lock['profile_file_sha256']:
        raise GateError('User model profiles changed since planning; review, do not overwrite')
    private = ROOT / '.local/runtime'
    private.mkdir(parents=True, exist_ok=True, mode=0o700)
    private.chmod(0o700)
    cache_path = private / 'hash-cache.json'
    cache = read_json(cache_path) if cache_path.exists() else {}
    models = list(lock['models'])
    model_roots = [Path('/home/alex/.ag1/agent/models'), Path('/home/alex/.lmstudio/models'), Path('/home/alex/.cache/llama.cpp')]
    if os.environ.get('AG1_LLAMA_MODELS_DIR'):
        model_roots.insert(0, Path(os.environ['AG1_LLAMA_MODELS_DIR']))
    ggufs = sorted({p.resolve() for base in model_roots if base.exists() for p in base.rglob('*.gguf')})
    weights = []
    for path in ggufs:
        matched = next((model for model in models if path.name == model + '.gguf' or path.name.startswith(model + '-000')), None)
        if not matched:
            continue  # Unselected MTP sidecars/projectors are not inferred to be in use.
        digest, reused = stable_hash(path, cache, force=args.force_hash)
        weights.append({'model': matched, 'path': str(path), 'sha256': digest,
                        'size': path.stat().st_size, 'signature': signature(path)})
        write_json(cache_path, cache)
        print(f'{"Verified cache" if reused else "Hashed"}: {path.name}', flush=True)
    missing = [m for m in models if not any(w['model'] == m for w in weights)]
    for model in models:
        shards = [w for w in weights if w['model'] == model]
        split = [re.search(r'-(\d{5})-of-(\d{5})\.gguf$', w['path']) for w in shards]
        split = [m for m in split if m]
        if split and ({int(m[1]) for m in split} != set(range(1, int(split[0][2]) + 1)) or len({m[2] for m in split}) != 1):
            raise GateError(f'Incomplete or inconsistent GGUF split set: {model}')
    system = loader_cache()
    libraries = {}; unresolved = []
    for model, row in lock['models'].items():
        profile = row['profile']
        own_dirs = [Path(p) for p in profile.get('libPath', '').split(':') if p]
        inherited_dirs = [Path(p) for p in os.environ.get('LD_LIBRARY_PATH', '').split(':') if p]
        queue = [Path(f['path']) for f in row['binary_and_adjacent_shared_libraries']]
        visited = set()
        while queue:
            path = queue.pop(0).resolve()
            if path in visited:
                continue
            visited.add(path)
            digest, _ = stable_hash(path, cache, force=args.force_hash)
            needed, search = elf_dynamic(path)
            resolved = []
            for name in needed:
                # Static closure candidate. Live loader mappings below are stronger evidence.
                choices = [d / name for d in own_dirs + inherited_dirs + [Path(x) for x in search]] + system.get(name, [])
                chosen = next((p.resolve() for p in choices if p.is_file()), None)
                if chosen:
                    resolved.append(str(chosen));queue.append(chosen)
                else:
                    unresolved.append({'model': model, 'object': str(path), 'needed': name})
            libraries[str(path)] = {'sha256': digest, 'size': path.stat().st_size,
                                    'needed': needed, 'resolved_candidates': resolved}
        for file in row['binary_and_adjacent_shared_libraries']:
            digest, _ = stable_hash(Path(file['path']), cache)
            if digest != file['sha256']:
                raise GateError(f'Pinned model binary/library changed: {file["path"]}')
    # Read only model process metadata. Private argv can contain paths or credentials.
    binaries = {str(Path(row['profile']['serverPath']).resolve()) for row in lock['models'].values()}
    observed = []; private_processes = []
    for process in Path('/proc').iterdir():
        if not process.name.isdigit():
            continue
        try:
            executable = str((process / 'exe').resolve(strict=True))
            if executable not in binaries:
                continue
            argv = (process / 'cmdline').read_bytes().split(b'\0')
            argv = [v.decode(errors='replace') for v in argv if v]
            maps = sorted({line.split()[-1] for line in (process / 'maps').read_text().splitlines()
                           if len(line.split()) >= 6 and line.split()[-1].startswith('/') and '.so' in line.split()[-1]})
            mapped = []
            for name in maps:
                p = Path(name)
                if p.is_file():
                    digest, _ = stable_hash(p, cache)
                    mapped.append({'path': name, 'sha256': digest})
            observed.append({'pid': int(process.name), 'executable': executable,
                             'argv_sha256': hashlib.sha256((process / 'cmdline').read_bytes()).hexdigest(),
                             'mapped_libraries': mapped, 'authority': 'observed external process; no control acquired'})
            private_processes.append({'pid': int(process.name), 'argv': argv})
        except (OSError, ProcessLookupError):
            continue
    write_json(private / 'observed-processes.json', private_processes)
    (private / 'observed-processes.json').chmod(0o600)
    # Preserve only explicitly relevant launch variables, never dump whole environments.
    allow = {k: v for k, v in os.environ.items() if k.startswith(('AG1_LLAMA_', 'LLAMA_CPP_')) or k == 'LD_LIBRARY_PATH'}
    write_json(private / 'launch-environment.json', allow)
    (private / 'launch-environment.json').chmod(0o600)
    write_json(cache_path, cache)
    report = {'version': 1, 'status': 'captured', 'profile_sha256': sha256(profile_path),
              'profiles': {m:r['profile'] for m,r in lock['models'].items()}, 'weights': weights,
              'missing_models': missing, 'loader_objects': libraries,
              'unresolved_static_dependencies': unresolved, 'observed_external_processes': observed,
              'launch_environment_keys': sorted(allow),
              'launch_environment_sha256': sha256(private / 'launch-environment.json'),
              'seconds': time.monotonic() - start, 'model_actions': {'spawn': 0, 'kill': 0, 'network': 0, 'tune': 0},
              'limitations': ['readelf closure is a static candidate; dlopen/loader precedence must be checked against per-model live maps before equivalence.',
                             'No projector inferred when absent; reference resolver capture determines selection.',
                             'Source HEAD/build directory names are not independently proved build provenance.',
                             'Request payload capture uses fake transport; no live model quality result.']}
    write_json(EVIDENCE / 'runtime-manifest.json', report)
    print(f'Captured {len(weights)} weight files, {len(libraries)} loader objects; {len(observed)} externally observed processes.')


if __name__ == '__main__':
    main()
