from pathlib import Path
import datetime
import hashlib
import json
import os
import shutil
import subprocess

root = Path('/home/alex/Projects/AG1')
out = root / 'implementation/evidence/storage-cleanup-20260913'
out.mkdir(exist_ok=True)
local = root / '.local'
candidates = [p for p in (local / 'p14-target').iterdir() if p.is_dir() and not p.is_symlink() and p.name != 'release']
for p in sorted(local.glob('*-target')):
    if p.name == 'p14-target' or p.is_symlink():
        continue
    if (p / 'CACHEDIR.TAG').is_file() and '8a477f597d28d172789f06886806bc55' in (p / 'CACHEDIR.TAG').read_text():
        candidates.append(p)
for name in ['.fingerprint', 'build', 'deps', 'incremental', 'examples']:
    p = root / 'product/target/debug' / name
    if p.exists():
        candidates.append(p)
packages = sorted((local / 'p14-packages').iterdir(), key=lambda p: p.stat().st_mtime, reverse=True)
kept_packages = packages[:2]
for p in packages[2:]:
    if p.is_dir() and not p.is_symlink() and len(p.name) == 32 and all(c in '0123456789abcdef' for c in p.name):
        candidates.append(p)
prefixes = tuple(str(p) + '/' for p in candidates)
exact = {str(p) for p in candidates}
for name in subprocess.check_output(['git', 'ls-files', '-z', '--', '.local', 'product/target'], cwd=root).split(b'\0'):
    if name:
        p = str(root / os.fsdecode(name))
        assert p not in exact and not p.startswith(prefixes), p
for p in candidates:
    assert p.is_dir() and not p.is_symlink() and p.resolve() == p, p
    assert p.stat().st_dev == root.stat().st_dev, p
active = []
for proc in Path('/proc').iterdir():
    if not proc.name.isdigit():
        continue
    refs = []
    for name in ['exe', 'cwd']:
        try:
            refs.append(os.readlink(proc / name))
        except OSError:
            pass
    try:
        for fd in (proc / 'fd').iterdir():
            try:
                refs.append(os.readlink(fd))
            except OSError:
                pass
    except OSError:
        pass
    try:
        refs.extend(line.split(maxsplit=5)[-1] for line in (proc / 'maps').read_text().splitlines())
    except OSError:
        pass
    active.extend({'pid': proc.name, 'path': ref} for ref in refs if ref in exact or ref.startswith(prefixes))
assert not active, active
sha = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
sources = {}
for base in ['product/crates', 'product/apps/desktop/src', 'tools']:
    for p in (root / base).rglob('*'):
        if p.is_file() and not p.is_symlink() and '__pycache__' not in p.parts:
            sources[str(p.relative_to(root))] = sha(p)
(out / 'source-sha256-before.json').write_text(json.dumps(sources, indent=2) + '\n')
runtime = json.loads(Path('/opt/ag1/current/external-runtimes.json').read_text())
protected = {str(p): sha(p) for p in [Path(runtime['profile']['path']), Path(runtime['active_supervisor']['path']), *[Path(p) for p in runtime['executables']]]}
(out / 'runtime-sha256-before.json').write_text(json.dumps(protected, indent=2) + '\n')
with (out / 'git-status-before').open('wb') as f:
    subprocess.run(['git', 'status', '--porcelain=v1', '-z'], cwd=root, stdout=f, check=True)
rows = [{'path': str(p), 'allocated_bytes': int(subprocess.check_output(['du', '-sx', '-B1', str(p)]).split()[0])} for p in candidates]
plan = {'created_utc': datetime.datetime.now(datetime.timezone.utc).isoformat(), 'home_free_before': os.statvfs('/home').f_bavail * os.statvfs('/home').f_frsize, 'remove': rows, 'keep_packages': [str(p) for p in kept_packages], 'source_file_count': len(sources), 'active_references_in_candidates': active}
(out / 'plan.json').write_text(json.dumps(plan, indent=2) + '\n')
print('Removing', len(rows), 'build/package directories; allocated GiB:', round(sum(r['allocated_bytes'] for r in rows) / 1024**3, 2), flush=True)
report = {'removed': [], 'failures': []}
for row in rows:
    try:
        shutil.rmtree(row['path'])
        report['removed'].append(row)
    except OSError as e:
        report['failures'].append({'path': row['path'], 'error': str(e)})
    (out / 'build-cleanup.json').write_text(json.dumps(report, indent=2) + '\n')
assert all(sha(root / p) == h for p, h in sources.items()), 'Source changed'
assert all(sha(Path(p)) == h for p, h in protected.items()), 'Protected runtime changed'
report['source_files_verified'] = len(sources)
report['protected_runtime_verified'] = True
report['home_free_after'] = os.statvfs('/home').f_bavail * os.statvfs('/home').f_frsize
(out / 'build-cleanup.json').write_text(json.dumps(report, indent=2) + '\n')
print(json.dumps({k: v for k, v in report.items() if k != 'removed'}, indent=2), flush=True)
