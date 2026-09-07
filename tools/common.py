"""Shared, fail-closed utilities for the implementation evidence tools."""
from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
ORIGINAL = Path('/home/alex/Projects/Cruzible')
PIN = 'e30f0b4b238ee5618928bcd17b4571eb046e7038'
DYAD_PIN = 'fb17039bfd2adcac7d74b9c1178a3a3524df35b6'
REFERENCE = ROOT / 'reference/ag1-executable'
EVIDENCE = ROOT / 'implementation/evidence/P01'


class GateError(RuntimeError):
    pass


def sha256(path: Path) -> str:
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def read_json(path: Path):
    try:
        return json.loads(path.read_text())
    except (OSError, ValueError) as exc:
        raise GateError(f'Cannot read JSON {path}: {exc}') from exc


def write_json(path: Path, data) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, temporary = tempfile.mkstemp(prefix=path.name + '.', dir=path.parent)
    try:
        with os.fdopen(fd, 'w') as stream:
            json.dump(data, stream, indent=2, ensure_ascii=False)
            stream.write('\n')
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
    finally:
        Path(temporary).unlink(missing_ok=True)


def run(command: list[str], *, cwd: Path = ROOT, log: Path, env=None, timeout=600):
    """No shell interpolation; preserve output, exact argv, failure and timing."""
    import time
    log.parent.mkdir(parents=True, exist_ok=True)
    start = time.monotonic()
    with log.open('w') as output:
        try:
            result = subprocess.run(command, cwd=cwd, env=env, stdout=output,
                                    stderr=subprocess.STDOUT, timeout=timeout)
        except subprocess.TimeoutExpired as exc:
            raise GateError(f'Command timed out; see {log}') from exc
    record = {'argv': command, 'cwd': str(cwd), 'exit_code': result.returncode,
              'seconds': time.monotonic() - start, 'log': str(log.relative_to(ROOT))}
    if result.returncode:
        raise GateError(f'Command failed ({result.returncode}); see {log}')
    return record


def isolated_env(directory: Path) -> dict[str, str]:
    """Do not inherit provider credentials, model overrides, or user config."""
    directory.mkdir(parents=True, exist_ok=True)
    for child in ['home', 'tmp', 'config', 'cache', 'agent']:
        (directory / child).mkdir(exist_ok=True)
    return {
        'PATH': os.environ.get('PATH', '/usr/bin:/bin'),
        'HOME': str(directory / 'home'), 'TMPDIR': str(directory / 'tmp'),
        'XDG_CONFIG_HOME': str(directory / 'config'),
        'XDG_CACHE_HOME': str(directory / 'cache'),
        'AG1_CODING_AGENT_DIR': str(directory / 'agent'),
        'AG1_LLAMA_CPP': '0', 'CI': '1', 'NO_COLOR': '1', 'TZ': 'UTC',
        'npm_config_offline': 'true', 'npm_config_fetch_retries': '0',
        'LANG': 'C.UTF-8',
    }


def verify_original() -> dict:
    head = subprocess.check_output(['git', '-C', str(ORIGINAL), 'rev-parse', 'HEAD'], text=True).strip()
    if head != PIN:
        raise GateError('AG1 HEAD differs from research pin; explicit delta review required')
    status = subprocess.check_output(['git', '--no-optional-locks', '-C', str(ORIGINAL), 'status', '--short'], text=True)
    if status != (ROOT / 'research/evidence/ag1-original-status.txt').read_text():
        raise GateError('Original status changed; preserve and review the delta')
    inventory = read_json(ROOT / 'research/evidence/ag1-source-inventory.json')
    for row in inventory:
        if sha256(ORIGINAL / row['path']) != row['sha256']:
            raise GateError(f'Original source fingerprint changed: {row["path"]}')
    dyad_head = subprocess.check_output(['git', '-C', str(ROOT / 'vendor/dyad'), 'rev-parse', 'HEAD'], text=True).strip()
    if dyad_head != DYAD_PIN:
        raise GateError('Dyad HEAD differs from research pin')
    if subprocess.check_output(['git', '--no-optional-locks', '-C', str(ROOT / 'vendor/dyad'), 'status', '--porcelain'], text=True):
        raise GateError('Dyad inspection checkout is no longer clean')
    return {'ag1_commit': head, 'dyad_commit': dyad_head,
            'source_modules_verified': len(inventory), 'original_status_unchanged': True}
