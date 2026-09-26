#!/usr/bin/env python3
"""Install a verified Linux AG1 package and its desktop icons under /opt and /usr."""
import argparse
import importlib.util
import os
from pathlib import Path
import shutil
import subprocess
import tempfile


def publish(path, data, mode=0o644):
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, temporary = tempfile.mkstemp(prefix=path.name + '.', dir=path.parent)
    try:
        with os.fdopen(fd, 'wb') as stream:
            stream.write(data)
            stream.flush()
            os.fsync(stream.fileno())
        os.chmod(temporary, mode)
        os.replace(temporary, path)
    finally:
        Path(temporary).unlink(missing_ok=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('archive', type=Path)
    parser.add_argument('sha256')
    args = parser.parse_args()
    if os.geteuid() != 0:
        parser.error('Run with sudo to install systemwide.')
    spec = importlib.util.spec_from_file_location('ag1_installer', Path(__file__).with_name('install.py'))
    installer = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(installer)
    destination = Path('/opt/ag1')
    release = installer.install(args.archive, args.sha256, destination)
    # The generic review installer starts private; shared application payloads
    # must be traversable by desktop users. User state stays in each user's home.
    destination.chmod(0o755)
    for size in (16, 24, 32, 48, 64, 128, 256, 512, 1024):
        publish(Path(f'/usr/share/icons/hicolor/{size}x{size}/apps/ag1.png'),
                (release / f'icons/{size}x{size}.png').read_bytes())
    publish(Path('/usr/local/bin/ag1-desktop'),
            b'#!/bin/sh\nexec /opt/ag1/current/run-ag1 "$@"\n', 0o755)
    publish(Path('/usr/share/applications/ag1.desktop'), b'''[Desktop Entry]
Name=AG1
Comment=AG1 Desktop
Exec=/usr/local/bin/ag1-desktop
Icon=ag1
Terminal=false
Type=Application
Categories=Development;
StartupNotify=true
StartupWMClass=Ag1-desktop
''')
    for command in (['update-desktop-database', '/usr/share/applications'],
                    ['gtk-update-icon-cache', '-f', '-t', '/usr/share/icons/hicolor']):
        if shutil.which(command[0]):
            subprocess.run(command, check=True)
    print(release)


if __name__ == '__main__':
    main()
