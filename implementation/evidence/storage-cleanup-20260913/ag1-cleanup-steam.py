from pathlib import Path
import datetime
import json
import os
import shutil
import subprocess

steam = Path('/home/alex/.var/app/com.valvesoftware.Steam')
backup = Path('/home/alex/steam-saves-backup-20260913')
out = Path('/home/alex/Projects/AG1/implementation/evidence/storage-cleanup-20260913')
assert steam.is_dir() and not steam.is_symlink() and steam.resolve() == steam
assert not backup.exists()
for proc in Path('/proc').iterdir():
    if not proc.name.isdigit():
        continue
    try:
        comm = (proc / 'comm').read_text().strip().lower()
    except OSError:
        continue
    assert comm not in ['steam', 'steamwebhelper', 'cs2', 'csgo_linux64'], (proc.name, comm)
free_before = os.statvfs('/home').f_bavail * os.statvfs('/home').f_frsize
size_before = int(subprocess.check_output(['du', '-sx', '-B1', str(steam)]).split()[0])
selected = [steam / p for p in ['.local/share/Steam/userdata', '.local/share/Steam/steamapps/compatdata', '.local/share/Steam/config', '.config', 'Pictures', 'Music']]
common = steam / '.local/share/Steam/steamapps/common'
for p in common.rglob('*'):
    if not p.is_symlink() and p.is_file() and p.suffix.lower() in ['.dem', '.cfg']:
        selected.append(p)
backup.mkdir(mode=0o700)
preserved = []
for p in selected:
    if not p.exists() or p.is_symlink():
        continue
    relative = p.relative_to(steam)
    dest = backup / relative
    dest.parent.mkdir(parents=True, exist_ok=True)
    inode = p.stat().st_ino
    p.rename(dest)
    assert dest.stat().st_ino == inode
    preserved.append(str(relative))
backup_bytes = int(subprocess.check_output(['du', '-sx', '-B1', str(backup)]).split()[0])
shutil.rmtree(steam)
assert not steam.exists()
report = {'time_utc': datetime.datetime.now(datetime.timezone.utc).isoformat(), 'removed_root': str(steam), 'allocated_bytes_before': size_before, 'backup': str(backup), 'backup_allocated_bytes': backup_bytes, 'preserved_paths': preserved, 'free_bytes_gained': os.statvfs('/home').f_bavail * os.statvfs('/home').f_frsize - free_before}
(out / 'steam-cleanup.json').write_text(json.dumps(report, indent=2) + '\n')
(backup / 'RESTORE.txt').write_text('Saved Steam user data, Proton prefixes, settings, game configuration files, and demos before removing Steam installation data on 2026-09-13.\nPaths here are relative to /home/alex/.var/app/com.valvesoftware.Steam. Restore after reinstalling Steam with Steam closed.\n')
print(json.dumps({k: v for k, v in report.items() if k != 'preserved_paths'}, indent=2))
