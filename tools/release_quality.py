#!/usr/bin/env python3
"""Execute preserved context-intelligence fixtures after strict canary admission."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import time

from common import ROOT, GateError, read_json, run, sha256, write_json
from release_gate import EVIDENCE, accepted_canary_digest, fingerprints, ref
from cutover_gate import validate_authority_audit
from p13_prepare_runtime import private_json
from tool_gate import environment


def main(output):
    os.umask(0o077)
    base=ROOT/'.local/p14-quality'
    base.mkdir(exist_ok=True,mode=0o700)
    output=output.absolute()
    if output.exists() or output.parent.resolve()!=base.resolve():
        raise GateError('Use a new direct child of .local/p14-quality')
    canary=read_json(EVIDENCE/'fresh-canaries.json')
    if canary.get('status')!='passed' or canary.get('checks')!=36 or canary.get('owned_runtime_events')!=48:
        raise GateError('Strict P14 live canaries must pass before broader quality replay')
    for item in canary['artifacts']:
        if sha256(ROOT/item['path'])!=item['sha256']:
            raise GateError('P14 canary evidence changed')
    audit_path=next(ROOT/r['path'] for r in canary['artifacts'] if r['path'].endswith('/audit.json'))
    request_path=next(ROOT/r['path'] for r in canary['artifacts'] if r['path'].endswith('/request.json'))
    cli=ROOT/'.local/p14-target/release/ag1-cutover'
    audit=read_json(audit_path)
    validate_authority_audit(audit,accepted_canary_digest(cli),read_json(ROOT/'implementation/evidence/P13/runtime-configuration-import.json'))
    inspected=subprocess.run([str(cli),'inspect','/home/alex/Projects/Cruzible'],capture_output=True,text=True,check=True,timeout=60)
    if json.loads(inspected.stdout)['authority']!=audit['authority']:
        raise GateError('Authority changed since fresh P14 production audit')
    # This read-only audit is an observation, not a lease. The native quality
    # owner obtains the shared runtime lease and freshly verifies all materials.
    node,env=environment()
    output.mkdir(mode=0o700)
    commands=[run([str(node),'tools/release_quality.mjs','capture',str(output)],env=env,log=output/'capture.log')]
    live=read_json(request_path)
    live['destination']=str(output/'matrix')
    private_json(output/'input.json',{'live':live,'cases':read_json(output/'cases.json')})
    binary=ROOT/'.local/p14-quality-target/release/examples/release-quality'
    before=fingerprints()
    command=[str(binary),str(output/'input.json')]
    private_json(output/'started.json',{'argv':command,'executable_sha256':sha256(binary),
                 'input':ref(output/'input.json'),'input_fingerprints':before,'automatic_retry':False})
    start=time.monotonic()
    with (output/'native.log').open('x') as log:
        completed=subprocess.run(command,stdout=log,stderr=subprocess.STDOUT,timeout=43200)
    private_json(output/'finished.json',{'exit_code':completed.returncode,'seconds':time.monotonic()-start,'started':ref(output/'started.json')})
    if completed.returncode:
        write_json(EVIDENCE/'context-intelligence.json',{'status':'failed','release_acceptance':False,'private_output':str(output.relative_to(ROOT)),
                   'started':ref(output/'started.json'),'finished':ref(output/'finished.json'),'log':ref(output/'native.log')})
        raise GateError('Native quality execution failed; owned settlement and private evidence retained')
    commands.append(run([str(node),'tools/release_quality.mjs','score',str(output)],env=env,log=output/'score.log'))
    quality=read_json(output/'quality.json')
    # Ignore fixture variant labels when comparing languages: each adapter ran both.
    scores=[]
    for row in quality['results']:
        variants=row['result']['results']
        scores.append({'model':row['model'],'runner':row['runner'],'successful_tasks':sum(v['observation']['taskSucceeded'] for v in variants),
                       'total_tasks':len(variants),'stale_claims':sum(v['observation']['staleAuthoritativeClaims'] for v in variants),
                       'unsupported_claims':sum(v['observation']['unsupportedAuthoritativeClaims'] for v in variants),
                       'parse_failures':sum('parseError' in v for v in variants)})
    artifacts=[ref(p) for p in sorted(output.rglob('*')) if p.is_file()]
    report={'status':'passed' if quality['all_tasks_succeeded'] else 'failed-quality','fixture':False,'release_acceptance':False,
            'scope':quality['scope'],'scores':scores,'commands':commands,'artifacts':artifacts,
            'input_fingerprints':before,'inputs_unchanged':fingerprints()==before}
    write_json(EVIDENCE/'context-intelligence.json',report)
    print(json.dumps(scores,indent=2))
    if not report['inputs_unchanged'] or report['status']!='passed':
        raise GateError('Quality/reproducibility failed; no release acceptance')


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('new_private_output',type=Path)
    main(parser.parse_args().new_private_output)
