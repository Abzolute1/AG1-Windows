import json
import os
import shutil
import subprocess
from pathlib import Path

base = Path('/opt/ag1')
current = (base / 'current').resolve()
receipt = json.loads((base / 'receipts' / (current.name + '.json')).read_text())
previous = (base / receipt['previous']).resolve()
assert not (base / 'pending.json').exists()
protected = {current, previous}
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
    for ref in refs:
        prefix = '/opt/ag1/releases/'
        if ref.startswith(prefix):
            protected.add(base / 'releases' / ref[len(prefix):].split('/')[0])
rows = []
for p in sorted((base / 'releases').iterdir()):
    if p in protected:
        continue
    assert p.is_dir() and not p.is_symlink()
    assert len(p.name) == 32 and all(c in '0123456789abcdef' for c in p.name)
    assert p.stat().st_dev == base.stat().st_dev
    manifest = json.loads((p / 'package-manifest.json').read_text())
    assert manifest['platform'] == 'linux-x86_64'
    assert (base / 'receipts' / (p.name + '.json')).is_file()
    rows.append({'path': str(p), 'allocated_bytes': int(subprocess.check_output(['du', '-sx', '-B1', str(p)]).split()[0])})
assert (base / 'current').resolve() == current
report = {'preserved': sorted(str(p) for p in protected), 'removed': []}
output = Path('/tmp/ag1-release-cleanup-result.json')
for row in rows:
    shutil.rmtree(row['path'])
    report['removed'].append(row)
    output.write_text(json.dumps(report, indent=2) + '\n')
assert (base / 'current').resolve() == current
print(json.dumps({'removed_releases': len(rows), 'reclaimed_bytes': sum(r['allocated_bytes'] for r in rows), 'preserved': report['preserved']}, indent=2))
