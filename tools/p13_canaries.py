"""Prepare a private P13 canary request and run its native runtime owner.

No existing model is adopted. A live window conflict exits with pending status;
the original TS owner and all user session data remain unchanged.
"""
from __future__ import annotations
import argparse
import datetime
import json
import subprocess
from pathlib import Path

from common import ROOT, REFERENCE, GateError, read_json, sha256, write_json, verify_original
from phase_gate import verify_dependencies
from runtime_gate import preservation
from reference_oracles import input_fingerprints
from p13_prepare_runtime import private_json


def run(capture_dir, request_path, new_destination):
    verify_dependencies('P13')
    pins = verify_original()
    before = preservation()
    reference = input_fingerprints()
    baseline = read_json(ROOT/'implementation/evidence/P01/completion.json')['input_fingerprints']
    if any(reference[key] != baseline[key] for key in ['dist','source_manifest','runtime_lock']):
        raise GateError('Executable TS reference differs from P01')
    capture = read_json(capture_dir/'configuration-receipt.json')
    if sha256(Path(capture['path'])) != capture['sha256']:
        raise GateError('Private configuration capture changed')
    private = (ROOT/'.local').resolve()
    request_path = request_path.absolute()
    new_destination = new_destination.absolute()
    if not request_path.parent.resolve().is_relative_to(private) or not new_destination.parent.resolve().is_relative_to(private):
        raise GateError('Use new private paths under .local')
    def material(path): return {'path':str(path), 'sha256':sha256(path), 'kind':'canary-host-input'}
    node = ROOT/'.local/toolchains/node-v24.14.0-linux-x64/bin/node'
    helper = ROOT/'product/target/release/ag1-process-host'
    cli = ROOT/'product/target/release/ag1-cutover'
    sources = sorted(REFERENCE.glob('packages/ai/dist/**/*.js'))
    sources += [REFERENCE/'package-lock.json', REFERENCE/'packages/ai/package.json']
    request = {'capture':capture,'models':read_json(capture_dir/'provider-models.json'),
               'process_host':material(helper),'destination':str(new_destination),
               'reference':{'node':material(node),'probe':material(ROOT/'tools/probes/p13_live_reference.mjs'),
                            'source':str(REFERENCE),'materials':[material(p) for p in sources]}}
    pinned = private_json(request_path, request)
    command = [str(cli),'live-canaries',str(request_path)]
    result = subprocess.run(command, capture_output=True, text=True, timeout=43200)
    after = preservation()
    if before != after or pins != verify_original(): raise GateError('Preservation changed during canaries')
    # The adapter's fail-closed resource observer runs before any model action.
    unavailable = result.returncode != 0 and 'external model/GPU process owns the runtime window' in result.stderr
    report = {'version':1,'observed_at':datetime.datetime.now(datetime.timezone.utc).isoformat(),
              'status':'passed' if result.returncode == 0 else 'pending' if unavailable else 'failed',
              'fixture':False,'command':command,'exit_code':result.returncode,
              'request':pinned,'importer_sha256':sha256(cli),'preservation_unchanged':True,
              'authority_transferred':False,'destination':str(new_destination),
              'model_actions':0 if unavailable else 'See private runtime events',
              'reason':'External runtime owner must release the exclusive window' if unavailable else
                       'See private canary report and owned process logs',
              'ts_dist_sha256':reference['dist']}
    private_json(request_path.with_suffix('.attempt.json'),report)
    write_json(ROOT/'implementation/evidence/P13/live-canary-attempt.json',report)
    print(json.dumps(report))
    return 0 if result.returncode == 0 else 2 if unavailable else 1


if __name__ == '__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('capture_directory',type=Path)
    parser.add_argument('new_private_request',type=Path)
    parser.add_argument('new_private_destination',type=Path)
    args=parser.parse_args()
    try: raise SystemExit(run(args.capture_directory,args.new_private_request,args.new_private_destination))
    except (GateError,OSError,ValueError,subprocess.SubprocessError):
        parser.exit(1,'P13 canary preparation failed; private evidence retained.\n')
