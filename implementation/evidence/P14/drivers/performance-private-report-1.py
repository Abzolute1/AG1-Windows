"""Run existing P14 measurements without overwriting historical public reports."""
import argparse
import json
import os
from pathlib import Path
import runpy
import sys

ROOT = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(ROOT / 'tools'))
from release_gate import artifact, input_digest, ref


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('kind', choices=['shell', 'context'])
    parser.add_argument('output', type=Path)
    parser.add_argument('--check-only', action='store_true')
    args = parser.parse_args()
    os.chdir(ROOT)
    os.umask(0o077)
    output = args.output.absolute()
    parent = ROOT / ('.local/p14-shell-pair' if args.kind == 'shell'
                     else '.local/p14-performance-preparation')
    assert output.parent == parent and not output.exists()
    reports = ROOT / '.local/p14-performance-private-reports' / output.name
    assert not reports.exists()
    public = ROOT / 'implementation/evidence/P14'
    names = ['shell-pair.json'] if args.kind == 'shell' else [
        'first-core-pair.json', 'core-performance.json']
    old = [ref(public / name) for name in names if (public / name).exists()]
    driver = ROOT / ('tools/release_shell_pair.py' if args.kind == 'shell'
                     else '.local/p14-performance-preparation/run_context.py')
    module = runpy.run_path(str(driver), run_name='private_p14_measurement')
    entry = module['main']
    original_write = entry.__globals__['write_json']

    def redirected_write(path, value):
        path = Path(path).absolute()
        if path.parent == public:
            assert path.name in names, 'Unreviewed public report write'
            path = reports / path.name
        return original_write(path, value)

    entry.__globals__['write_json'] = redirected_write
    if args.kind == 'context':
        assert entry.__globals__['EVIDENCE'] == public
        # This prepared target has no prior context executable. Do not overwrite
        # a subsequently measured binary on a later invocation of this wrapper.
        assert not (ROOT / '.local/p14-followup-target/release/context-probe').exists()
        entry.__globals__['EVIDENCE'] = output
    digest = input_digest()
    if args.check_only:
        print('Report routing checked; no measurement, build or report write')
        return
    reports.mkdir(parents=True, mode=0o700)
    original_write(reports / 'routing.json', {
        'kind': args.kind, 'input_sha256': digest,
        'driver': ref(driver), 'wrapper': ref(Path(__file__).resolve()),
        'prior_reports': old, 'output': str(output.relative_to(ROOT)),
        'scope': 'Original measurement commands and criteria; private report routing only.',
        'duration_workload': False,
    })
    try:
        if args.kind == 'shell':
            entry(output)
        else:
            entry()
    finally:
        for binding in old:
            artifact(binding)
        assert input_digest() == digest


if __name__ == '__main__':
    main()
