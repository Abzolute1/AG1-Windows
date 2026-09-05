"""P07 executable state, source-race and durable evidence gate."""
import datetime,hashlib,json,os,shutil,tempfile
from pathlib import Path
from common import ROOT,PIN,GateError,read_json,write_json,sha256,run,verify_original
from provider_gate import validate_rust_log

def validate_measurements(rust,ts):
    for value in [rust,ts]:
        if value.get('status')!='measured' or value.get('model_actions')!=0 or value.get('network_actions')!=0:raise GateError('Invalid measurement authority/status')
        if not value.get('scope') or value.get('state_entries')!=50 or value.get('state_bytes',0)<=0:raise GateError('Unbounded or missing state benchmark')
        samples=value.get('serialization_ns',[])
        if len(samples)!=100 or any(type(n)is not int or n<=0 for n in samples):raise GateError('Incomplete state timings')
        rows=value.get('files',[])
        if [r.get('bytes') for r in rows]!=[1024,1048576,8388608]:raise GateError('Missing source size benchmark')
        for row in rows:
            expected=hashlib.sha256(b'a'*row['bytes']).hexdigest()
            if row.get('sha256')!=expected:raise GateError('Source bytes differ')
            for key in ['snapshot_ns','hash_ns','repeated_validation_ns']:
                samples=row.get(key,[])
                if len(samples)!=5 or any(type(n)is not int or n<=0 for n in samples):raise GateError('Incomplete source timings')
    if rust['state_bytes']!=ts['state_bytes'] or rust.get('state_sha256')!=ts.get('state_sha256'):raise GateError('State output mismatch')

def inputs():
    files=[]
    for base in ['product/crates/ag1-core','product/crates/ag1-platform','product/crates/ag1-storage','product/licenses','fixtures/evidence']:
        files.extend(p for p in (ROOT/base).rglob('*') if p.is_file())
    files.extend(ROOT/p for p in ['product/Cargo.toml','product/Cargo.lock','phases/P07-task-and-evidence-state.md','tools/evidence_gate.py','tools/provider_gate.py','tools/tests/test_evidence_gate.py','tools/phase_gate.py','tools/common.py','tools/record_protocol_licenses.py','tools/probes/evidence_reference.mjs','tools/probes/evidence_measure_reference.mjs','tools/offline_guard.mjs'])
    return {str(p.relative_to(ROOT)):sha256(p) for p in sorted(set(files))}

def execute_p07():
    from phase_gate import verify_dependencies,verify_artifacts,verify_runtime_material
    evidence=ROOT/'implementation/evidence/P07';evidence.mkdir(parents=True,exist_ok=True);completion=evidence/'completion.json'
    write_json(completion,{'phase':'P07','status':'running'});commands=[]
    try:
        verify_dependencies('P07');original=verify_original()
        node=ROOT/'.local/toolchains/node-v24.14.0-linux-x64/bin/node';env=os.environ.copy();env['PATH']=str(node.parent)+':'+env.get('PATH','/usr/bin:/bin');env['NO_COLOR']='1'
        def execute(name,argv):
            print('P07: '+name,flush=True);commands.append(run(argv,log=evidence/f'{name}.log',env=env,timeout=1200))
        execute('licenses',['python3','tools/record_protocol_licenses.py']);before=inputs()
        execute('gate-tests',['python3','-m','unittest','discover','-s','tools/tests','-p','test_evidence_gate.py','-v'])
        execute('reference',[str(node),'tools/probes/evidence_reference.mjs'])
        oracle=read_json(evidence/'reference.log')
        if oracle!={'status':'passed','cases':94,'model_actions':0,'network_actions':0}:raise GateError('Incomplete state oracle execution')
        fixture=read_json(ROOT/'fixtures/evidence/state-oracle-v2.json')
        if fixture['source_commit']!=PIN or len(fixture['rows'])!=94:raise GateError('Unpinned state oracle')
        execute('format',['cargo','fmt','--manifest-path','product/Cargo.toml','--all','--','--check'])
        execute('clippy',['cargo','clippy','--manifest-path','product/Cargo.toml','-p','ag1-core','-p','ag1-platform','-p','ag1-storage','--all-targets','--','-D','warnings'])
        counts={}
        for package,suite,count in [('ag1-core','evidence_contract',9),('ag1-platform','source_contract',7),('ag1-storage','evidence_storage_contract',6),('ag1-storage','storage_contract',28),('ag1-platform','platform_contract',27),('ag1-core','provider_contract',10)]:
            name=package+'-'+suite;execute(name,['cargo','test','--manifest-path','product/Cargo.toml','-p',package,'--test',suite]);counts[name]=validate_rust_log((evidence/f'{name}.log').read_text(),count)
        execute('build',['cargo','build','--manifest-path','product/Cargo.toml','-p','ag1-platform','--release','--bin','evidence-probe'])
        with tempfile.TemporaryDirectory(prefix='p07-source-',dir=ROOT/'.local') as temporary:
            for size in [1024,1048576,8388608]:Path(temporary,f'source-{size}.txt').write_bytes(b'a'*size)
            state=str(ROOT/'fixtures/evidence/measurement-state.json')
            execute('rust-measure',[str(ROOT/'product/target/release/evidence-probe'),temporary,state,str(evidence/'rust-measurements.json')])
            execute('ts-measure',[str(node),'tools/probes/evidence_measure_reference.mjs',temporary,state,str(evidence/'ts-measurements.json')])
        validate_measurements(read_json(evidence/'rust-measurements.json'),read_json(evidence/'ts-measurements.json'))
        verify_original();verify_dependencies('P07');verify_runtime_material(read_json(ROOT/'implementation/evidence/P01/runtime-manifest.json'),read_json(ROOT/'.local/runtime/hash-cache.json'))
        lock=read_json(ROOT/'planning/evidence/llama-runtime-lock.json')
        if sha256(Path(lock['profile_file']))!=lock['profile_file_sha256']:raise GateError('Original llama profile changed')
        after=inputs()
        if before!=after:raise GateError('State inputs changed during gate')
        limitations=[
            'Shadow task/memory/evidence services only; the usable desktop remains TS-owned until P13. No model, GPU, desktop or project runtime was launched.',
            'Linux source capability/race and storage tests executed. Windows/macOS/WASM and actual native visual/runtime integration remain later gates.',
            'Source reads are limited to 8 MiB and reread full bytes for freshness. Only parsed references are cached; metadata-only shortcuts are intentionally excluded.',
            'Memory selection preserves the reference soft character budget, including one bounded first entry that may exceed it. The sixteen-entry cap stays hard. The shadow catalog is limited to 10000 retained memories and 4 MiB documents.',
            'Task and observation text caps preserve complete Unicode scalar values at UTF-16 boundaries. Runtime observation timestamps require valid RFC3339; informal date strings are not accepted as timestamps.',
            'Historical invalid/duplicate task identity and corrupt memory fail visibly and retain the P05 original archive. Dangling executive links cannot acquire authority during migration.',
            'A26 planning state remains distinct; interactive plan/question/mode policy is P10/P12. A39 prompt insertion is P08 and A45 actual native visual capture/attachment is P11.',
            'TS benchmarks hash the same warm-cache files but its cached validator has weaker metadata assumptions and its source revision API does not capture display text. No equivalent-correctness speed or desktop footprint claim is made.',
            'The initial state oracle is retained for audit. The v2 oracle explicitly parses persisted task state before mutation, matching the Rust boundary; both were generated through the unchanged SDK.',
            'Existing tuned llama.cpp binaries, libraries, weights and profiles were verified unchanged. No model settings or process ownership were transferred.'
        ]
        coverage={'contracts':counts,'frozen_state_cases':94,'capabilities':['A22','A23','A24','A25','A27','A36','A38-state','A39-selection','A40','A41','A42','A43','A44','A45-metadata'],'invariants':['I06','I07','I08','I10','I11','I12','I13','I14'],'defects':['B03'],'runtime_preservation':['L01','L02','L03','L04','L05','L06','L07','L08'],'model_actions':0,'desktop_launches':0}
        write_json(evidence/'coverage.json',coverage);write_json(evidence/'limitations.json',limitations)
        (evidence/'REPORT.md').write_text('# P07 task, memory and byte-consistent evidence\n\nThe gate executed '+str(sum(counts.values()))+' Rust contracts, the 94-case unchanged SDK state corpus, strict validator tests and matched-input measurements. See [completion authority](completion.json), [coverage](coverage.json) and [the state API guide](delivered-snapshot/product/crates/ag1-core/STATE.md).\n\nB03 tests force mutations after reading and before validation: same-size edits, replacement/rename, symlink swaps and root replacement cannot produce mismatched displayed bytes and digest. BOM, CRLF, Unicode, clipping, binary and over-limit cases remain explicit. P05 storage corruption, fork closure and crash-recovery contracts were rerun.\n\nThe phase keeps objective and steering distinct, validates claim/plan transitions, persists independent observation records, retains archived memory and enforces requester authorization for evidence pages/cursors. No executing desktop or inference authority changed.\n\n'+ '\n'.join('- '+s for s in limitations)+'\n')
        for relative in after:
            target=evidence/'delivered-snapshot'/relative;target.parent.mkdir(parents=True,exist_ok=True);shutil.copyfile(ROOT/relative,target)
        artifacts=[{'path':str(p.relative_to(ROOT)),'sha256':sha256(p)} for p in sorted(evidence.rglob('*')) if p.is_file() and p!=completion and p.name!='gate.log'];verify_artifacts(ROOT,artifacts)
        write_json(completion,{'phase':'P07','status':'passed','completed_at':datetime.datetime.now(datetime.timezone.utc).isoformat(),'dependencies':['P03','P05'],'input_fingerprints':after,'commands':commands,'artifacts':artifacts,'source_verification':original,'coverage':coverage,'authority':'Independent Rust shadow state and evidence. TS remains executing desktop/session authority.','pending_gates':limitations,'rollback':'Discard private shadow destinations; preserve immutable originals and prior checkpoints. Never overwrite live journals or model material.'})
        print(json.dumps({'phase':'P07','status':'passed','contracts':sum(counts.values()),'model_actions':0}))
    except Exception as exc:write_json(completion,{'phase':'P07','status':'failed','error':str(exc),'commands':commands});raise
