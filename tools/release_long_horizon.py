#!/usr/bin/env python3
"""Prepare and run the original live long-horizon comparison in disposable state."""
import argparse
import copy
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time
from common import ROOT, REFERENCE, GateError, read_json, sha256, write_json, verify_original
from release_gate import artifact, ref, fingerprints
from p13_prepare_legacy import inventory
from tool_gate import environment


def material(path):
    return {'path': str(Path(path).absolute()), 'sha256': sha256(Path(path)), 'kind': 'p14-workload-input'}


def native_input(ts, destination, live, model, fixture=False):
    """Rebase only a derived fixture journal header; retain all source bytes."""
    seed=read_json(ts/'seed.json')
    if not seed['token_counts_exact'] or not seed['raw_token_target_met'] or not seed['artifacts_intact']:
        raise GateError('Exact seed calibration/integrity did not pass')
    destination.mkdir(mode=0o700)
    workspace=destination/'workspace'; workspace.mkdir(mode=0o700)
    source=destination/'seed-state'
    original=Path(seed['source'])
    if not original.resolve().is_relative_to((ROOT/'.local/p14-long-horizon').resolve()):
        raise GateError('Seed must belong to private long-horizon work')
    if any(p.is_symlink() for p in original.rglob('*')):
        raise GateError('Seed links are not an implicit closure grant')
    shutil.copytree(original, source)
    journal=source/Path(seed['journal']).name
    original_bytes=journal.read_bytes(); header,history=original_bytes.split(b'\n',1)
    value=json.loads(header); value['cwd']=str(workspace)
    journal.write_bytes(json.dumps(value,separators=(',',':')).encode()+b'\n'+history)
    for path in original.rglob('*'):
        if path.is_file() and path.name!=journal.name:
            if sha256(path)!=sha256(source/path.relative_to(original)):
                raise GateError('Derived seed changed an artifact')
    seed.update(workspace=str(workspace),source=str(source),journal=str(journal))
    write_json(destination/'seed.json',seed)
    manifest,inspected=inventory(workspace,source,seed['session'])
    write_json(destination/'manifest.json',manifest)
    write_json(destination/'rebase.json',{'original_journal_sha256':hashlib.sha256(original_bytes).hexdigest(),
        'derived_journal':ref(journal),'unchanged_history_sha256':hashlib.sha256(history).hexdigest(),
        'changed_field':'session header cwd only','all_artifact_bytes_unchanged':True,'fixture':fixture})
    node,env=environment()
    generated=read_json(ts/('input.json' if fixture else 'worker-input.json'))['workload']['generated']
    subprocess.run([str(node),'tools/release_long_horizon_tasks.mjs',generated['path'],generated['sha256'],str(ts/'seed.json'),str(destination/'tasks.json')],check=True,env=env,timeout=60)
    tasks=read_json(destination/'tasks.json');write_json(destination/'specs.json',tasks['specs'])
    settings={'defaultThinkingLevel':'low' if model['reasoning'] else 'off','contextWindow':model['contextWindow'],
              'compaction':{'enabled':False},'retry':{'enabled':True,'maxRetries':1,'baseDelayMs':250,'maxDelayMs':2000}}
    write_json(destination/'settings.json',settings)
    config={'models':live['models'],'selected':model['id'],'credential_env':None,'system_prompt':tasks['system_prompt'],
        'fixture':fixture,'policy_mode':'disabled','max_generations':None,
        'imported':{'settings':{'path':str(destination/'settings.json'),'sha256':sha256(destination/'settings.json')},'credentials':None},
        'runtime':None if fixture else {'capture':live['capture'],'process_host':live['process_host']},
        'tooling':{'process_host':live['process_host']['path'],'git':'/usr/bin/git','rg':shutil.which('rg'),'fd':'/usr/bin/fd','bubblewrap':'/usr/bin/bwrap',
                   'targets':{'node':None,'npm':None,'cargo':None,'rustc':None,'godot':None,'mounts':[]}}}
    request={'destination':str(destination),'seed':material(destination/'seed.json'),'manifest':material(destination/'manifest.json'),
             'specs':material(destination/'specs.json'),'node':material(Path('/usr/bin/node')),'config':config}
    write_json(destination/'input.json',request)
    return destination/'input.json'


def run(output):
    os.umask(0o077)
    base=ROOT/'.local/p14-long-horizon';base.mkdir(exist_ok=True,mode=0o700)
    output=output.absolute()
    if output.exists() or output.parent.resolve()!=base.resolve():raise GateError('New direct child of .local/p14-long-horizon required')
    pins=verify_original()
    # Read-only accepted inputs; the owner still acquires real custody and rehashes materials.
    live=copy.deepcopy(read_json(ROOT/'.local/p14-quality/diagnosis-live-2/input.json')['live'])
    canaries=read_json(ROOT/'implementation/evidence/P14/fresh-canaries.json')
    if canaries['status']!='passed':raise GateError('Strict canary admission missing')
    for row in canaries['artifacts']:artifact(row)
    cli=ROOT/'.local/p14-target/release/ag1-cutover'
    inspected=subprocess.run([str(cli),'inspect','/home/alex/Projects/Cruzible'],capture_output=True,text=True,check=True,timeout=60)
    authority=json.loads(inspected.stdout)['authority']
    if authority!=read_json(ROOT/'implementation/evidence/P14/final-preservation-inspection.json')['authority']['authority']:
        raise GateError('Accepted quiescent production authority changed')
    model=next(m for m in live['models'] if m['id']=='Qwen3.6-35B-A3B-UD-Q4_K_XL')
    generated=ROOT/'.local/p14-long-horizon-preparation-3/fixture-generator.mjs'
    preparation=read_json(generated.parent/'preparation.json')
    if sha256(generated)!=preparation['generated_sha256'] or sha256(Path(preparation['source']))!=preparation['source_sha256']:
        raise GateError('Original fixture extraction changed')
    output.mkdir(mode=0o700)
    live['destination']=str(output/'ts')
    live['reference']['probe']=material(ROOT/'tools/release_long_horizon_worker.mjs')
    files=sorted(REFERENCE.glob('packages/*/dist/**/*.js'))+[REFERENCE/'package-lock.json',REFERENCE/'package.json']+sorted(REFERENCE.glob('packages/*/package.json'))+[generated]
    live['reference']['materials']=[material(p) for p in files]
    request={'live':live,'model':model['id'],'workload':{'mode':'seed-and-typescript','rawTokens':2500000,'phases':64,'generated':material(generated)}}
    write_json(output/'input.json',request)
    binary=ROOT/'.local/p14-quality-target/release/examples/release-long-horizon-owner'
    native=ROOT/'.local/p14-quality-target/release/examples/release-long-horizon-native'
    before=fingerprints()
    write_json(output/'started.json',{'input':ref(output/'input.json'),'owner':ref(binary),'native':ref(native),'input_fingerprints':before,
        'authority':authority,'fixture':False,'release_acceptance':False,'duration_run':False,
        'scope':'Original six-task live preflight, TS then native, with 2.5M exactly tokenized externalized history. No automatic retry.',
        'project_tool_node':material(Path('/usr/bin/node')),'ts_interpreter':live['reference']['node']})
    results=[]
    start=time.monotonic()
    with (output/'owner.log').open('x') as log:
        completed=subprocess.run([str(binary),str(output/'input.json')],stdout=log,stderr=subprocess.STDOUT,timeout=14400)
    results.append({'runner':'typescript','exit_code':completed.returncode,'seconds':time.monotonic()-start})
    execution=read_json(output/'ts/execution.json')
    if not execution['runtime_settled'] or not execution['preservation_verified']:
        raise GateError('TS owner did not settle; native admission forbidden')
    # A failed TS repair remains failed evidence. The independent Rust comparator
    # may proceed only after proved settlement and a valid immutable seed.
    request_path=native_input(output/'ts',output/'native',live,model)
    start=time.monotonic()
    with (output/'native.log').open('x') as log:
        completed=subprocess.run([str(native),str(request_path)],stdout=log,stderr=subprocess.STDOUT,timeout=14400)
    results.append({'runner':'rust','exit_code':completed.returncode,'seconds':time.monotonic()-start})
    write_json(output/'finished.json',{'runs':results,'inputs_unchanged':before==fingerprints(),'original_pins_unchanged':pins==verify_original(),'release_acceptance':False})
    print(json.dumps(results,indent=2))
    if any(r['exit_code'] for r in results):raise GateError('Live preflight retained a failure; no acceptance')


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('new_private_output',type=Path)
    run(parser.parse_args().new_private_output)
