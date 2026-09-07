"""Capture read-only runtime configuration into a NEW private directory.

The observed PID is evidence only. This tool neither acquires control of that
process nor admits inference or workspace mutation. Raw environment/vault
material stays in the ignored private directory; stdout is a redacted receipt.
Build AG1's importer first with cargo build --offline --release
--manifest-path product/Cargo.toml -p ag1-cli --bin ag1-cutover.
"""
from __future__ import annotations
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import subprocess

from common import ROOT, GateError, read_json, sha256, verify_original
from phase_gate import verify_dependencies
from runtime_gate import preservation
from tool_gate import environment


def private_json(path, value):
    data = (json.dumps(value, sort_keys=True, separators=(',', ':'))+'\n').encode()
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(fd, 'wb') as stream:
        stream.write(data)
        stream.flush()
        os.fsync(stream.fileno())
    return {'path': str(path), 'sha256': hashlib.sha256(data).hexdigest()}


def observation(pid):
    proc = Path('/proc')/str(pid)
    before = (proc/'stat').read_bytes()
    executable = os.readlink(proc/'exe')
    raw_argv = (proc/'cmdline').read_bytes()
    raw_environment = (proc/'environ').read_bytes()
    after = (proc/'stat').read_bytes()
    # starttime is field 22; the parenthesized comm may contain spaces.
    start = lambda value: value.rsplit(b')', 1)[1].split()[19]
    if start(before) != start(after) or os.readlink(proc/'exe') != executable:
        raise GateError('Observed runtime changed during read-only capture')
    pairs = [value.decode().split('=', 1) for value in raw_environment.split(b'\0') if value]
    if len(dict(pairs)) != len(pairs):
        raise GateError('Duplicate process environment requires explicit resolution')
    return {'pid': pid, 'start_ticks': start(before).decode(), 'executable': executable,
            'argv': [value.decode() for value in raw_argv.split(b'\0') if value],
            'environment': dict(pairs), 'control_granted': False}


def prepare(destination, pid):
    verify_dependencies('P13')
    original = verify_original()
    preserved = preservation()
    lock = read_json(ROOT/'planning/evidence/llama-runtime-lock.json')
    bridge = read_json(ROOT/'.local/bridge-runtime.json')
    profile = Path(bridge['profile_file'])
    if sha256(profile) != lock['profile_file_sha256'] or bridge['profile_sha256'] != lock['profile_file_sha256']:
        raise GateError('Profile drift')
    if set(bridge['models']) != set(lock['models']):
        raise GateError('Incomplete preserved model inventory')
    observed = observation(pid)
    matches = [(key, row['profile']) for key, row in lock['models'].items()
               if row['profile']['serverPath'] == observed['executable']]
    if len(matches) != 1:
        raise GateError('Observed executable does not identify exactly one pinned profile')
    observed_model, observed_profile = matches[0]
    inherited = observed['environment'].copy()
    # Undo exactly one documented resolver library prepend, then prove the
    # roundtrip against the complete observed argv AND environment below.
    library = observed_profile.get('libPath')
    if library:
        value = inherited.get('LD_LIBRARY_PATH', '')
        if value == library:
            del inherited['LD_LIBRARY_PATH']
        elif value.startswith(library+':'):
            inherited['LD_LIBRARY_PATH'] = value[len(library)+1:]
        else:
            raise GateError('Observed library environment cannot roundtrip the pinned resolver')
    if observed_profile.get('environment'):
        raise GateError('Parent environment cannot be inferred across profile environment overrides')
    destination = destination.absolute()
    # Private launch evidence must remain under ignored local storage.
    local = (ROOT/'.local').resolve()
    if not destination.parent.resolve().is_relative_to(local):
        raise GateError('Use a new directory under ignored .local storage')
    destination.mkdir(mode=0o700)
    private_json(destination/'observed-runtime.json', observed)
    inherited_pin = private_json(destination/'inherited-environment.json', inherited)
    gpu = subprocess.run(['/usr/bin/nvidia-smi', '--query-gpu=memory.total', '--format=csv,noheader,nounits'],
                         check=True, capture_output=True, text=True, timeout=10)
    memory = gpu.stdout.strip().splitlines()
    if len(memory) != 1: raise GateError('Select a single preserved GPU before capture')
    request = {'profile': str(profile), 'environment': inherited, 'vram_mib': int(memory[0]),
               'models': {key: row['path'] for key, row in bridge['models'].items()}}
    private_json(destination/'ts-request.json', request)
    _, env = environment()
    subprocess.run(['node', str(ROOT/'tools/probes/p13_runtime_capture.mjs'), str(ROOT/'reference/ag1-executable'),
                    str(destination/'ts-request.json'), str(destination/'ts-capture.json')],
                   env=env, check=True, capture_output=True, timeout=120)
    captured = read_json(destination/'ts-capture.json')
    row = next(row for row in captured['captures'] if row['model_id'] == observed_model)
    if [row['command'], *row['args']] != observed['argv'] or row['environment'] != observed['environment']:
        raise GateError('Pinned TS resolution differs from the observed runtime; private capture retained')
    if observation(pid) != observed:
        raise GateError('Observed runtime changed before configuration publication')
    executables = {row['profile']['serverPath'] for row in lock['models'].values()}
    materials = [{'path': row['path'], 'sha256': row['sha256'],
                  'kind': 'executable' if row['path'] in executables else
                          ('projector' if 'mmproj' in Path(row['path']).name.lower() else 'weight')
                          if row['path'].endswith('.gguf') else 'library'} for row in bridge['material']]
    additional = {}
    for name, filename in [('settings', 'settings.json'), ('credentials', 'auth.json')]:
        path = profile.parent/filename
        if path.exists(): additional[name] = {'path': str(path), 'sha256': sha256(path)}
    cpu_pairs = set()
    for block in Path('/proc/cpuinfo').read_text().split('\n\n'):
        fields = dict(line.split(':', 1) for line in block.splitlines() if ':' in line)
        fields = {key.strip(): value.strip() for key, value in fields.items()}
        if 'physical id' in fields and 'core id' in fields:
            cpu_pairs.add((fields['physical id'], fields['core id']))
    if not cpu_pairs: raise GateError('Cannot verify CPU topology')
    imports = []
    for row in captured['captures']:
        args = row['args']
        imports.append({'model_id': row['model_id'], 'model_path': row['model_path'],
                        'context_override': None, 'forced_moe': None, 'cached_moe': None,
                        'discovered_projector': args[args.index('--mmproj')+1] if '--mmproj' in args else None,
                        'materials': materials, 'expected': {key: row[key] for key in
                            ['command', 'args', 'environment', 'context_window', 'max_tokens']}})
    private_json(destination/'import-request.json', {
        'profile': {'path': str(profile), 'sha256': sha256(profile)},
        'inherited_environment': inherited_pin, 'additional_inputs': additional,
        'physical_cores': len(cpu_pairs), 'logical_cores': len(os.sched_getaffinity(0)),
        'vram_mib': int(memory[0]), 'models': imports,
    })
    command = [str(ROOT/'product/target/release/ag1-cutover'), 'capture-runtime',
               str(destination/'import-request.json'), str(destination/'configuration.json')]
    result = subprocess.run(command, check=True, capture_output=True, text=True, timeout=1800)
    capture = json.loads(result.stdout)
    private_json(destination/'configuration-receipt.json', capture)
    private_json(destination/'provider-models.json', [row['provider_model'] for row in captured['captures']])
    if original != verify_original() or preserved != preservation():
        raise GateError('Original or runtime preservation changed during capture')
    receipt = {'version': 1, 'observed_at': datetime.datetime.now(datetime.timezone.utc).isoformat(),
               'status': 'offline_configuration_verified', 'fixture': False,
               'profile_sha256': sha256(profile), 'plan_sha256': capture['plan_sha256'],
               'configuration_sha256': capture['sha256'], 'private_directory': str(destination),
               'importer_sha256': sha256(Path(command[0])), 'import_command': command,
               'observed_pid': pid, 'observed_runtime_roundtrip': True, 'process_control_granted': False,
               'model_actions': 0, 'live_acceptance': False, 'originals_preserved': True,
               'scope': 'All three exact TS/Rust plans; observed model argv/environment roundtrip. No live canary or transfer.'}
    private_json(destination/'receipt.json', receipt)
    return receipt


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('new_private_directory', type=Path)
    parser.add_argument('--observe-pid', type=int, required=True)
    args = parser.parse_args()
    try:
        print(json.dumps(prepare(args.new_private_directory, args.observe_pid)))
    except (GateError, OSError, ValueError, KeyError, StopIteration, subprocess.SubprocessError):
        # Avoid echoing private request/environment values from child errors.
        parser.exit(1, 'P13 runtime capture failed; private evidence retained, no authority or model action granted.\n')
