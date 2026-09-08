#!/usr/bin/env python3
"""Fresh matched TS/Rust context measurements; P14 outputs, no prior gate writes."""
import tempfile
import uuid

from common import ROOT, GateError, read_json, run, sha256, write_json
from context_gate import compare_measurements
from release_gate import EVIDENCE, fingerprints
from tool_gate import environment


def main():
    out=EVIDENCE/'core-measurements'/uuid.uuid4().hex
    out.mkdir(parents=True)
    node,env=environment()
    env['CARGO_TARGET_DIR']=str(ROOT/'.local/p14-target')
    before=fingerprints()
    commands=[run(['cargo','build','--offline','--locked','--manifest-path','product/Cargo.toml',
                   '-p','ag1-storage','--release','--bin','context-probe'],env=env,log=out/'build.log',timeout=1800)]
    binary=ROOT/'.local/p14-target/release/context-probe'
    binary_sha=sha256(binary)
    with tempfile.TemporaryDirectory(prefix='p14-context-',dir=ROOT/'.local') as temporary:
        for index in range(3):
            tasks=[('rust',[str(binary),temporary,str(out/f'rust-{index}.json'),'--timing']),
                   ('typescript',[str(node),'tools/probes/context_measure_reference.mjs',str(out/f'typescript-{index}.json')])]
            if index%2:tasks.reverse()
            for name,argv in tasks:
                commands.append(run(argv,env=env,log=out/f'{name}-{index}.log',timeout=1200))
            # Publish the first pair immediately for inspection; it is not a noise-qualified pass.
            if index==0:
                write_json(EVIDENCE/'first-core-pair.json',{'scope':'First matched context pair; 25 samples/workload; not noise-qualified',
                           'rust':str((out/'rust-0.json').relative_to(ROOT)),
                           'typescript':str((out/'typescript-0.json').relative_to(ROOT))})
    rust=[read_json(out/f'rust-{i}.json') for i in range(3)]
    ts=[read_json(out/f'typescript-{i}.json') for i in range(3)]
    comparison=compare_measurements(rust,ts)
    if fingerprints()!=before or sha256(binary)!=binary_sha:
        raise GateError('Core measurement input or executable changed')
    comparison.update(binary_sha256=binary_sha,input_fingerprints=before,commands=commands,
                      release_acceptance=False,concurrent_load='Shared workstation; see per-run spread. This is not packaged application performance.')
    write_json(EVIDENCE/'core-performance.json',comparison)
    print('Fresh matched context measurements:', 'within budget' if comparison['passed'] else 'regression detected')
    if not comparison['passed']:
        raise GateError('A matched context p95 exceeds the 10% regression budget')


if __name__=='__main__':main()
