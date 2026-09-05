"""P08 executable differential, durability and measurement gates. Never changes TS authority."""
import datetime, json, os, shutil, statistics, tempfile, uuid
from pathlib import Path
from common import ROOT, PIN, GateError, read_json, write_json, sha256, run, verify_original
from provider_gate import validate_rust_log

PROBES = {
 'compiler': {'compiler_cases':16}, 'source': {'source_cases':8},
 'episode': {'records':13,'searches':8,'candidates':6,'features':5},
 'prompt': {'prompts':20,'prefixes':5},
 'projection': {'tasks':15,'memories':9,'inspections':4,'runtimes':2,'capsules':12},
 'compaction': {'histories':8,'horizons':18,'summaries':2,'ledgers':4},
 'signals': {'recent':5,'trees':4}, 'summary': {'groups':9,'fixture_provider_calls':10},
 'draft': {'groups':3,'fixture_model_calls':2}, 'retry': {'retry_cases':32,'snip_cases':8},
 'addenda': {'plans':16,'turns':6},
}
CONTRACTS = [('ag1-core','context_contract',12),('ag1-platform','retrieval_contract',3),('ag1-storage','context_storage_contract',6)]

def validate_probe(name, data):
    expected={'status':'passed', **PROBES[name], 'model_actions':0,'network_actions':0}
    if data != expected: raise GateError(f'Incomplete or changed context reference execution: {name}: {data}')

def inputs():
    files=[]
    for base in ['product/crates/ag1-core','product/crates/ag1-platform','product/crates/ag1-storage','product/licenses','fixtures/context']:
        files.extend(p for p in (ROOT/base).rglob('*') if p.is_file())
    files.extend(ROOT/p for p in ['product/Cargo.toml','product/Cargo.lock','phases/P08-retrieval-and-context.md','tools/context_gate.py','tools/provider_gate.py','tools/tests/test_context_gate.py','tools/phase_gate.py','tools/reference_oracles.py','tools/common.py','tools/record_protocol_licenses.py','tools/offline_guard.mjs'])
    files.extend((ROOT/'tools/probes').glob('context_*.mjs'))
    return {str(p.relative_to(ROOT)):sha256(p) for p in sorted(set(files))}

def environment():
    node=ROOT/'.local/toolchains/node-v24.14.0-linux-x64/bin/node'
    env=os.environ.copy();env['PATH']=str(node.parent)+':'+env.get('PATH','/usr/bin:/bin');env['NO_COLOR']='1'
    return node,env

def differential():
    from reference_oracles import input_fingerprints
    evidence=ROOT/'implementation/evidence/P08';evidence.mkdir(parents=True,exist_ok=True)
    output=evidence/'oracle-context-differential.json';commands=[];run_id=uuid.uuid4().hex
    before={'product':inputs(),'reference':input_fingerprints()}
    write_json(output,{'status':'running','run_id':run_id});node,env=environment()
    try:
        counts={}
        for name in PROBES:
            log=evidence/f'reference-{name}.log'
            commands.append(run([str(node),f'tools/probes/context_{name}_reference.mjs'],env=env,log=log,timeout=1200))
            data=read_json(log);validate_probe(name,data);counts[name]=data
            fixture=read_json(ROOT/f'fixtures/context/{name}-oracle.json')
            if fixture.get('source_commit') != PIN or fixture.get('model_actions') != 0 or fixture.get('network_actions') != 0: raise GateError('Unpinned or live context oracle')
        tests={}
        for package,suite,count in CONTRACTS:
            log=evidence/f'{suite}.log'
            commands.append(run(['cargo','test','--manifest-path','product/Cargo.toml','-p',package,'--test',suite],env=env,log=log,timeout=1200))
            tests[suite]=validate_rust_log(log.read_text(),count)
        after={'product':inputs(),'reference':input_fingerprints()}
        if before != after: raise GateError('Context inputs changed during differential replay')
        report={'status':'passed','run_id':run_id,'suite':'context-differential','input_fingerprints':after,'commands':commands,'reference_probes':counts,'contracts':tests,'model_actions':0,'network_actions':0}
        write_json(output,report);print(json.dumps({'status':'passed','reference_probes':len(counts),'contracts':sum(tests.values()),'model_actions':0}))
        return report
    except Exception as exc:
        write_json(output,{'status':'failed','run_id':run_id,'commands':commands,'error':str(exc)});raise

HISTORIES=['32k-pressure','protected-201','externalized-2m-tokens']
def percentile(samples, p=.95):
    import math
    return sorted(samples)[math.ceil(len(samples)*p)-1]

def validate_measurement(data, runtime, *, allocations=False):
    if data.get('status')!='measured' or data.get('runtime')!=runtime or data.get('model_actions')!=0 or data.get('network_actions')!=0 or not data.get('scope'): raise GateError('Invalid context measurement authority')
    if data.get('samples')!=25 or [g.get('id') for g in data.get('groups',[])]!=HISTORIES: raise GateError('Missing context workloads')
    if runtime=='rust' and data.get('allocator_instrumentation') is not allocations: raise GateError('Instrumented timings cannot certify runtime comparison')
    oracle={g['id']:g for g in read_json(ROOT/'fixtures/context/compiler-oracle.json')['groups']}
    for g in data['groups']:
        expected=oracle[g['id']]
        if g.get('input_sha256')!=expected['inputSha256']: raise GateError('Mismatched benchmark input')
        for pass_ in ['cold','warm']:
            if g.get(pass_+'_stats')!=expected[pass_]['stats']:raise GateError('Benchmark quality changed')
            # Rust serde and canonical SDK objects use sorted JSON keys and compact UTF-8.
            import hashlib
            digest=hashlib.sha256(json.dumps(expected[pass_]['messages'],sort_keys=True,separators=(',',':'),ensure_ascii=False).encode()).hexdigest()
            if g.get(pass_+'_sha256')!=digest:raise GateError('Selected evidence/order changed')
        for pass_ in ['cold','warm','rehydration']:
            rows=g.get(pass_,[])
            if len(rows)!=25 or any(type(r.get('ns')) is not int or r['ns']<=0 for r in rows):raise GateError('Missing or invalid timing samples')
            if allocations and any(type(r.get(k)) is not int or r[k]<0 for r in rows for k in ['allocation_calls','requested_allocation_bytes','peak_extra_live_bytes']):raise GateError('Missing allocator observations')
        if g.get('artifacts',0)<=0 or g.get('artifact_bytes',0)<=0:raise GateError('No recoverable benchmark evidence')
        if runtime=='rust':
            d=g.get('durable',{})
            if d.get('original_sha256')!=g['input_sha256'] or d.get('original_bytes')!=g.get('input_bytes') or d.get('logical_file_bytes',0)<=d['original_bytes'] or d.get('physical_files',0)<=0:raise GateError('Missing durable original custody')
            for key in ['cold','warm','reopen','rehydrate_original']:
                if type(d.get(key,{}).get('ns')) is not int or d[key]['ns']<=0:raise GateError('Missing durable recovery measurements')

def compare_measurements(rust, ts):
    if len(rust)!=3 or len(ts)!=3:raise GateError('Three alternating-order runs required for noise characterization')
    for r in rust:validate_measurement(r,'rust')
    for r in ts:validate_measurement(r,'typescript')
    rows=[]
    for i,id_ in enumerate(HISTORIES):
        for pass_ in ['cold','warm','rehydration']:
            for a,b in zip(rust,ts):
                for key in ['input_bytes','input_messages','input_sha256','cold_sha256','warm_sha256','artifact_bytes','artifacts']:
                    if a['groups'][i].get(key)!=b['groups'][i].get(key):raise GateError('Unequal timing workload: '+key)
            a=[s['ns'] for r in rust for s in r['groups'][i][pass_]];b=[s['ns'] for r in ts for s in r['groups'][i][pass_]]
            ap=percentile(a);bp=percentile(b);ratio=ap/bp
            spread={}
            for name,runs in [('rust',rust),('typescript',ts)]:
                p95=[percentile([s['ns'] for s in r['groups'][i][pass_]]) for r in runs]
                spread[name]={'run_p95_ns':p95,'max_over_min':max(p95)/min(p95)}
            row={'history':id_,'pass':pass_,'samples_per_runtime':75,'rust_p95_ns':ap,'typescript_p95_ns':bp,'ratio':ratio,'noise':spread,'within_10_percent':ratio<=1.1};rows.append(row)
    return {'scope':'Equivalent volatile artifact callbacks and exact selected evidence. Rust cold includes owned immutable capture validation and cached token estimates; warm shares those immutable bytes and refreshes leases/artifact access. No filesystem durability claim or device-cache eviction. P05 durable rows are separate and not compared to unsynced TS storage. All samples retained, including initial JIT/GC; nearest-rank p95 pooled across three alternating-order runs. Per-run spread records observed noise, not a statistical confidence interval. Allocator instrumentation runs separately.','rows':rows,'passed':all(r['within_10_percent'] for r in rows)}

def execute_p08():
    from phase_gate import verify_dependencies,verify_artifacts,verify_runtime_material
    evidence=ROOT/'implementation/evidence/P08';evidence.mkdir(parents=True,exist_ok=True);completion=evidence/'completion.json'
    write_json(completion,{'phase':'P08','status':'running'});commands=[]
    try:
        verify_dependencies('P08');original=verify_original();node,env=environment()
        def execute(name,argv):
            print('P08: '+name,flush=True);commands.append(run(argv,log=evidence/f'{name}.log',env=env,timeout=1200))
        execute('licenses',['python3','tools/record_protocol_licenses.py']);before=inputs()
        execute('gate-tests',['python3','-m','unittest','discover','-s','tools/tests','-p','test_context_gate.py','-v'])
        execute('format',['cargo','fmt','--manifest-path','product/Cargo.toml','--all','--','--check'])
        execute('clippy',['cargo','clippy','--manifest-path','product/Cargo.toml','-p','ag1-core','-p','ag1-platform','-p','ag1-storage','--all-targets','--','-D','warnings'])
        execute('differential',['python3','tools/reference_oracles.py','--suite','context-differential','--offline'])
        oracle=read_json(evidence/'oracle-context-differential.json')
        if oracle.get('status')!='passed' or oracle.get('input_fingerprints',{}).get('product')!=before:raise GateError('Stale or failed differential replay')
        counts=dict(oracle['contracts'])
        for package,suite,count in [('ag1-core','evidence_contract',9),('ag1-platform','source_contract',7),('ag1-storage','evidence_storage_contract',6),('ag1-storage','storage_contract',28),('ag1-core','provider_contract',10)]:
            execute(suite,['cargo','test','--manifest-path','product/Cargo.toml','-p',package,'--test',suite]);counts[suite]=validate_rust_log((evidence/f'{suite}.log').read_text(),count)
        execute('measure-build',['cargo','build','--manifest-path','product/Cargo.toml','-p','ag1-storage','--release','--bin','context-probe'])
        binary=ROOT/'product/target/release/context-probe';binary_hash=sha256(binary)
        with tempfile.TemporaryDirectory(prefix='p08-context-',dir=ROOT/'.local') as temporary:
            for i in range(3):
                work=[('rust',[str(binary),temporary,str(evidence/f'rust-timing-{i}.json'),'--timing']),('typescript',[str(node),'tools/probes/context_measure_reference.mjs',str(evidence/f'typescript-timing-{i}.json')])]
                if i%2:work.reverse()
                for name,argv in work:execute(f'{name}-timing-{i}',argv)
            execute('allocations',[str(binary),temporary,str(evidence/'rust-allocations.json')])
        if sha256(binary)!=binary_hash:raise GateError('Measurement executable changed')
        rust=[read_json(evidence/f'rust-timing-{i}.json') for i in range(3)];ts=[read_json(evidence/f'typescript-timing-{i}.json') for i in range(3)]
        comparison=compare_measurements(rust,ts);comparison['binary_sha256']=binary_hash
        write_json(evidence/'performance.json',comparison)
        validate_measurement(read_json(evidence/'rust-allocations.json'),'rust',allocations=True)
        if not comparison['passed']:raise GateError('Context p95 regression exceeds 10%; inspect performance.json and retained noise samples')
        verify_original();verify_dependencies('P08');verify_runtime_material(read_json(ROOT/'implementation/evidence/P01/runtime-manifest.json'),read_json(ROOT/'.local/runtime/hash-cache.json'))
        lock=read_json(ROOT/'planning/evidence/llama-runtime-lock.json')
        if sha256(Path(lock['profile_file']))!=lock['profile_file_sha256']:raise GateError('Original llama profile changed')
        after=inputs()
        if before!=after:raise GateError('Context inputs changed during gate')
        limitations=[
            'TS remains executing desktop/session authority until P13. P08 delivers Rust context/retrieval shadow services and durable projection receipts; live admission, compaction epochs, retries and sequential isolated calls integrate in P10.',
            'No desktop, real model, GPU, foreign process or credential-dependent provider was launched. Optional drafts and summary requests use actual SDK fixture providers; Rust requests bind the P06 same-model identity and existing scheduler permit contract.',
            'Snapshot estimates are cached only on private immutable owned messages; every pass refreshes artifact authorization and leases. Source freshness still rereads validated bytes. The compatibility Vec<Value> API materializes owned output and is not the hot-path timing API.',
            'All 201 protected messages remain recoverable; protected over-target diagnostics are accepted without deleting protected evidence. Large input contains over two million estimated tokens; all selected evidence and raw output hashes are compared unchanged.',
            'Source host runs owned Linux rg or a bounded CPU fallback. Git/recent signals consume explicit captured state. Filesystem tie ordering is deterministic path order; source metadata never proves freshness.',
            'Historical dates require RFC3339 and UTF-16 clipping retains complete Unicode scalars. Capsule display paths must match source-reference paths; malformed paths are rejected. Pair-boundary correction can retain more history than the legacy SDK cut.',
            'Legacy PTL head-drop is a stateless analysis transform; durable pinned user intent/task state and raw trajectory are retained separately. Controlled storage projections reject orphan tool results and reject a changed journal revision before returning.',
            'Summary preparation JSON omits JavaScript undefined properties at the persisted JSON boundary. Random artifact IDs/clocks alone are normalized; selection, ordering, claim kinds, full prompts and raw evidence digests are unchanged.',
            'Compiler/retrieval measurements are synthetic offline workloads, not whole-application or model-throughput results. P05 durable custody/reopen/page timings and retained file sizes are reported separately; no physical device-write or universal fsync guarantee is claimed.',
            'Windows/macOS, real native/visual integration, live per-model quality and 8h/24h release workloads remain explicit P11–P14 gates. The three tuned llama.cpp builds, linked libraries, GGUFs and original profiles remain unchanged.'
        ]
        coverage={'contracts':counts,'reference_probes':oracle['reference_probes'],'capabilities':['A15','A18','A19','A20','A21','A28','A29','A30','A31','A32','A33','A34','A35','A39','A76'],'invariants':['I02','I05','I09','I12','I28'],'defects':['B03 freshness correction preserved'],'runtime_preservation':['L01','L02','L03','L04','L05','L06','L07','L08'],'model_actions':0,'desktop_launches':0}
        write_json(evidence/'coverage.json',coverage);write_json(evidence/'limitations.json',limitations)
        (evidence/'REPORT.md').write_text('# P08 retrieval, episodes and context compilation\n\nThe gate executed '+str(sum(counts.values()))+' Rust contracts, eleven unchanged SDK probes, validator rejection tests, strict Clippy/formatting and three alternating-order measurement runs per runtime. See [completion authority](completion.json), [performance and noise](performance.json), [coverage](coverage.json), and [the API guide](delivered-snapshot/product/crates/ag1-core/CONTEXT.md).\n\nOriginal request and journal custody must acknowledge durable bytes before a reduction runs. Failure injection covers blob sync, rename, transaction, commit and acknowledgement boundaries; restart retains uncertain evidence, rejects orphans and detects concurrent journal revisions. Episode close retains original trajectory bytes, branches preserve evidence authorization, and candidate drafting remains non-authoritative.\n\n'+ '\n'.join('- '+s for s in limitations)+'\n')
        for relative in after:
            target=evidence/'delivered-snapshot'/relative;target.parent.mkdir(parents=True,exist_ok=True);shutil.copyfile(ROOT/relative,target)
        artifacts=[{'path':str(p.relative_to(ROOT)),'sha256':sha256(p)} for p in sorted(evidence.rglob('*')) if p.is_file() and p!=completion and p.name!='gate.log'];verify_artifacts(ROOT,artifacts)
        write_json(completion,{'phase':'P08','status':'passed','completed_at':datetime.datetime.now(datetime.timezone.utc).isoformat(),'dependencies':['P06','P07'],'input_fingerprints':after,'commands':commands,'artifacts':artifacts,'source_verification':original,'coverage':coverage,'authority':'Rust shadow context services; TS executing desktop/session unchanged. P10 owns final admission and epoch integration.','pending_gates':limitations,'rollback':'Discard isolated shadow destinations; keep exact custody artifacts, original journals and prior TS checkpoint. No live migration or model tuning occurred.'})
        print(json.dumps({'phase':'P08','status':'passed','contracts':sum(counts.values()),'reference_probes':len(PROBES),'model_actions':0}))
    except Exception as exc:
        write_json(completion,{'phase':'P08','status':'failed','commands':commands,'error':str(exc)});raise
