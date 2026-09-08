#!/usr/bin/env python3
"""Fresh strict TS/Rust canaries under P14 custody; never refresh P13 evidence.

These are transport/configuration regressions, not the broader P14 quality suite.
The native executable alone holds the shared runtime lease and controls its children.
"""
import argparse
import datetime
import os
from pathlib import Path
import subprocess
import time

from common import ROOT, REFERENCE, GateError, read_json, sha256, write_json
from cutover_gate import validate_authority_audit, validate_live_canaries, validate_live_configuration
from release_gate import EVIDENCE, ref, accepted_canary_digest
from p13_prepare_runtime import private_json


def run(output):
    from phase_gate import verify_dependencies
    verify_dependencies('P14')
    os.umask(0o077)
    output = output.absolute()
    if output.exists() or not output.parent.resolve().is_relative_to((ROOT/'.local').resolve()) or output.parent.is_symlink():
        raise GateError('Use a new private P14 output under .local')
    output.mkdir(mode=0o700)
    cli = ROOT/'.local/p14-target/release/ag1-cutover'
    capture_dir = ROOT/'.local/p13-runtime-capture-20260907'
    capture = read_json(capture_dir/'configuration-receipt.json')
    configuration = read_json(ROOT/'implementation/evidence/P13/runtime-configuration-import.json')
    accepted_digest = accepted_canary_digest(cli)
    # A fresh audit immediately precedes this invocation. A saved pass is not custody.
    start = time.monotonic()
    audit = subprocess.run([str(cli),'audit-cutover','/home/alex/Projects/Cruzible'],capture_output=True,text=True,timeout=1800)
    private_json(output/'audit-command.json',{'exit_code':audit.returncode,'seconds':time.monotonic()-start,'stderr':audit.stderr})
    if audit.returncode:
        raise GateError('Fresh native authority/preservation audit refused live testing; private audit-command.json retained')
    import json
    value = json.loads(audit.stdout)
    private_json(output/'audit.json',value)
    validate_authority_audit(value, accepted_digest, configuration)
    def material(path):
        return {'path':str(path),'sha256':sha256(path),'kind':'canary-host-input'}
    node = ROOT/'.local/toolchains/node-v24.14.0-linux-x64/bin/node'
    source = sorted(REFERENCE.glob('packages/ai/dist/**/*.js'))
    source += [REFERENCE/'package-lock.json',REFERENCE/'packages/ai/package.json']
    request = {'capture':capture,'models':read_json(capture_dir/'provider-models.json'),
               'process_host':material(ROOT/'product/target/release/ag1-process-host'),
               'destination':str(output/'matrix'),
               'reference':{'node':material(node),'probe':material(ROOT/'tools/probes/p13_live_reference.mjs'),
                            'source':str(REFERENCE),'materials':[material(p) for p in source]}}
    private_json(output/'request.json',request)
    command = [str(cli),'live-canaries',str(output/'request.json')]
    private_json(output/'started.json',{'argv':command,'executable_sha256':sha256(cli),
                 'started_at':datetime.datetime.now(datetime.timezone.utc).isoformat(),
                 'request':ref(output/'request.json'),'automatic_retry':False})
    start = time.monotonic()
    with (output/'stdout.json').open('x') as out, (output/'stderr.log').open('x') as err:
        completed = subprocess.run(command,stdout=out,stderr=err,timeout=43200)
    private_json(output/'finished.json',{'exit_code':completed.returncode,'seconds':time.monotonic()-start,'started':ref(output/'started.json')})
    artifacts = [ref(p) for p in sorted(output.iterdir()) if p.is_file()]
    matrix_path = output/'matrix/canaries.json'
    report = {'status':'failed','phase':'P14','scope':'Fresh six-case per-model transport/configuration regression; broader quality/duration acceptance remains pending',
              'phase_complete':False,'exit_code':completed.returncode,'artifacts':artifacts}
    if matrix_path.is_file():
        matrix = read_json(matrix_path)
        report['artifacts'].append(ref(matrix_path))
        if completed.returncode == 0:
            validate_live_canaries(matrix['report'], [m['id'] for m in matrix['models']])
            validate_live_configuration(matrix,configuration)
            report['status']='passed'
            report['checks']=len(matrix['report']['rows'])
            report['owned_runtime_events']=len(matrix['runtime_events'])
    write_json(EVIDENCE/'fresh-canaries.json',report)
    if report['status']!='passed':
        raise GateError('Fresh live canaries failed; all private evidence retained')
    print('P14 strict canaries passed; broader release gates remain separate')


if __name__ == '__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('new_private_output',type=Path)
    run(parser.parse_args().new_private_output)
