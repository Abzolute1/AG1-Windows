"""P06: executed provider parity, owned lifecycle, credentials and bounded scheduling."""
import collections, datetime, hashlib, json, os, re, shutil
from pathlib import Path
from common import ROOT, PIN, GateError, read_json, write_json, sha256, run, verify_original

FAMILIES = {'anthropic-messages','openai-completions','mistral-conversations','openai-responses','azure-openai-responses','openai-codex-responses','google-generative-ai','google-gemini-cli','google-vertex','bedrock-converse-stream'}

def validate_rust_log(log, minimum):
    summaries = re.findall(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured; (\d+) filtered out;', log)
    if len(summaries) != 1: raise GateError('Missing or ambiguous contract summary')
    passed, *other = map(int, summaries[0])
    cases = re.findall(r'^test ([a-zA-Z_][a-zA-Z0-9_:]*) \.\.\. ok$', log, re.M)
    if passed < minimum or any(other) or len(cases) != passed or len(set(cases)) != passed: raise GateError('Empty, skipped or incomplete contracts')
    return passed

def validate_groups(value, repeats=1):
    if value.get('source_commit') != PIN or value.get('network_actions') != 0 or value.get('model_actions',0) != 0: raise GateError('Unpinned or effectful oracle')
    counts = collections.Counter(row.get('api') for row in value.get('groups',[]))
    if counts != collections.Counter({api:repeats for api in FAMILIES}): raise GateError('Provider child coverage incomplete')
    if repeats > 1 and len({(r['api'],r.get('scenario')) for r in value['groups']}) != 10*repeats: raise GateError('Duplicate provider scenario')

def validate_measurements(rust, ts, workload):
    raw = (workload['fragment']*workload['fragments']).encode()
    for value in [rust,ts]:
        if value.get('status') != 'measured' or value.get('model_actions') != 0 or value.get('network_actions') != 0: raise GateError('Missing or effectful measurement')
        if value.get('workload') != workload or value.get('output_bytes') != len(raw) or value.get('output_sha256') != hashlib.sha256(raw).hexdigest(): raise GateError('Nonidentical benchmark output/workload')
        if value.get('peak_waiters') != 128 or value.get('max_rss_kib',0) <= 0 or not value.get('scope'): raise GateError('Unbounded or undocumented benchmark')
        for key, count in [('normalization_ns',5),('queue_batch_ns',100)]:
            samples=value.get(key,[])
            if len(samples)!=count or any(type(n) is not int or n<=0 for n in samples): raise GateError('Incomplete benchmark timings')
    if rust.get('wire_bytes',0)<=0 or rust['wire_bytes']!=ts.get('wire_bytes'): raise GateError('Nonidentical wire workload')

def inputs():
    files=[]
    for base in ['product/crates/ag1-core','product/crates/ag1-platform','product/licenses','fixtures/providers']:
        files.extend(p for p in (ROOT/base).rglob('*') if p.is_file())
    files.extend((ROOT/'tools/probes').glob('provider*'))
    files.extend(ROOT/p for p in ['tools/probes/llama_discovery_reference.mjs','tools/offline_guard.mjs','product/Cargo.toml','product/Cargo.lock','product/apps/desktop/src-tauri/src/lib.rs','product/apps/desktop/src-tauri/tests/bridge_contract.rs','phases/P06-providers-and-preserved-llama.md','tools/provider_gate.py','tools/tests/test_provider_gate.py','tools/phase_gate.py','tools/common.py','tools/record_protocol_licenses.py'])
    return {str(p.relative_to(ROOT)):sha256(p) for p in sorted(set(files)) if p.is_file()}

def execute_p06():
    from phase_gate import verify_dependencies, verify_artifacts, verify_runtime_material
    evidence=ROOT/'implementation/evidence/P06'; evidence.mkdir(parents=True,exist_ok=True)
    completion=evidence/'completion.json'; write_json(completion,{'phase':'P06','status':'running'}); commands=[]
    try:
        verify_dependencies('P06'); original=verify_original()
        verify_artifacts(ROOT,read_json(ROOT/'implementation/evidence/P01/completion.json')['artifacts'])
        node=ROOT/'.local/toolchains/node-v24.14.0-linux-x64/bin/node'
        env=os.environ.copy();env['PATH']=str(node.parent)+':'+env.get('PATH','/usr/bin:/bin');env['NO_COLOR']='1'
        commands.append(run(['python3','tools/record_protocol_licenses.py'],log=evidence/'licenses.log',env=env))
        before=inputs()
        def execute(name, argv):
            print('P06: '+name,flush=True)
            commands.append(run(argv,log=evidence/f'{name}.log',env=env,timeout=1200))
        execute('gate-tests',['python3','-m','unittest','discover','-s','tools/tests','-p','test_provider_gate.py','-v'])
        for probe in ['provider_reference','provider_request_reference','provider_options_reference','provider_headers_reference','provider_oauth_reference','provider_aws_reference','provider_capability_reference','llama_discovery_reference','provider_vision_reference']:
            execute(probe,[str(node),f'tools/probes/{probe}.mjs'])
        for file,repeats in [('oracle',1),('headers',1),('requests',3),('options',3)]: validate_groups(read_json(ROOT/f'fixtures/providers/{file}.json'),repeats)
        for file,key,count in [('capabilities','profiles',96),('capabilities','thinking',42),('vision','groups',42),('discovery','pure',16),('discovery','tuning',28),('oauth','groups',4),('aws','groups',3),('aws','serialized',3)]:
            data=read_json(ROOT/f'fixtures/providers/{file}.json')
            if data.get('source_commit')!=PIN or len(data.get(key,[]))!=count: raise GateError('Incomplete '+file+' oracle')
        execute('format',['cargo','fmt','--manifest-path','product/Cargo.toml','--all','--','--check'])
        execute('clippy',['cargo','clippy','--manifest-path','product/Cargo.toml','-p','ag1-core','-p','ag1-platform','--all-targets','--','-D','warnings'])
        counts={}
        for package,suite,minimum in [('ag1-core','provider_contract',10),('ag1-core','model_contract',3),('ag1-platform','lib',3),('ag1-platform','llama_contract',12),('ag1-platform','provider_wire_contract',7),('ag1-platform','provider_http_contract',11),('ag1-platform','provider_auth_contract',6),('ag1-platform','provider_request_contract',5),('ag1-platform','provider_capability_contract',1),('ag1-platform','platform_contract',27),('ag1-desktop','bridge_contract',3)]:
            name=package+'-'+suite
            execute(name,['cargo','test','--manifest-path','product/Cargo.toml','-p',package]+(['--lib'] if suite=='lib' else ['--test',suite]))
            counts[name]=validate_rust_log((evidence/f'{name}.log').read_text(),minimum)
        execute('build',['cargo','build','--manifest-path','product/Cargo.toml','-p','ag1-platform','--release','--bin','provider-probe'])
        execute('rust-measure',[str(ROOT/'product/target/release/provider-probe'),str(evidence/'rust-measurements.json')])
        execute('ts-measure',[str(node),'tools/probes/provider_measure_reference.mjs',str(evidence/'ts-measurements.json')])
        validate_measurements(read_json(evidence/'rust-measurements.json'),read_json(evidence/'ts-measurements.json'),read_json(ROOT/'fixtures/providers/measurement-workload.json'))
        verify_original(); verify_dependencies('P06')
        verify_runtime_material(read_json(ROOT/'implementation/evidence/P01/runtime-manifest.json'),read_json(ROOT/'.local/runtime/hash-cache.json'))
        lock=read_json(ROOT/'planning/evidence/llama-runtime-lock.json')
        if sha256(Path(lock['profile_file']))!=lock['profile_file_sha256']:raise GateError('Original runtime profile changed')
        after=inputs()
        if before!=after: raise GateError('Provider inputs changed during gate')
        limitations=[
            'Rust remains a shadow provider/runtime adapter. The usable desktop retains TS scheduler and runtime authority until P13.',
            'All provider traffic is synthetic or loopback fixture traffic. Cloud credential canaries, interactive OAuth login and live per-model quality/TTFT/prefill/decode/VRAM are pending P13/P14; none is counted passed.',
            'Owned process and storage contracts ran on Linux. Windows/macOS/WASM execution and release packaging remain pending.',
            'Credential refresh persists through the private P05 store. This is not an OS keychain or encrypted-at-rest claim. Command-derived credentials require the later P09 broker.',
            'Google OAuth refresh at the source pin contains placeholder client credentials and explicitly returns unavailable; valid existing tokens and project envelopes remain supported.',
            'Benchmarks share exact fake bytes, fragment rate and queue depth. TS includes SDK request/Promise setup; Rust measures framing/domain reduction and admission. RSS includes different runtime dependencies. No desktop or live-model speed comparison is claimed.',
            'Pure versioned tuning proposals do not apply settings. No tuning, live model launch, profile rewrite, backend rebuild, port eviction or process ownership adoption occurred.',
            'Native HTTP uses a declared AG1 Rust user agent and host headers; Node SDK telemetry/browser-permission headers are intentionally omitted. Dynamic extension providers and user-facing auth integration belong to P09/P12.'
        ]
        write_json(evidence/'limitations.json',limitations)
        coverage={'contracts':counts,'provider_families':sorted(FAMILIES),'request_cases':70,'preserved_local_requests':6,'preserved_launch_plans':9,'capability_cases':96,'thinking_cases':42,'vision_cases':42,'discovery_cases':16,'tuning_cases':28,'oauth_refresh_cases':4,'sigv4_cases':3,'bedrock_http_cases':3,'model_actions':0,'desktop_launches':0}
        write_json(evidence/'coverage.json',coverage)
        (evidence/'REPORT.md').write_text('# P06 providers and preserved llama.cpp\n\nThe executed gate covers all ten pinned provider families, 70 request captures, six preserved local payloads, nine exact launch plans, bounded FIFO admission, quarantine after uncertain cancellation, durable credential refresh, and owned Linux runtime lifecycle. The [completion record](completion.json) contains executed commands and immutable delivered hashes.\n\nNo desktop or live model was launched. The existing tuned binaries, libraries, weights and profile bytes were verified unchanged. Rust adapters remain shadow services until P13.\n\nThe [API guide](delivered-snapshot/product/crates/ag1-platform/PROVIDERS.md) describes provider, isolated-call, credential and process boundaries. [Coverage](coverage.json), [Rust measurements](rust-measurements.json) and [TS measurements](ts-measurements.json) record the actual run. Measurements share identical synthetic output bytes; setup costs and runtime dependencies differ.\n\n'+ '\n'.join('- '+s for s in limitations)+'\n')
        snapshot=evidence/'delivered-snapshot'
        for relative in after:
            target=snapshot/relative; target.parent.mkdir(parents=True,exist_ok=True);shutil.copyfile(ROOT/relative,target)
        artifacts=[{'path':str(p.relative_to(ROOT)),'sha256':sha256(p)} for p in sorted(evidence.rglob('*')) if p.is_file() and p!=completion and p.name!='gate.log']
        verify_artifacts(ROOT,artifacts)
        write_json(completion,{'phase':'P06','status':'passed','completed_at':datetime.datetime.now(datetime.timezone.utc).isoformat(),'dependencies':['P03','P05'],'input_fingerprints':after,'commands':commands,'artifacts':artifacts,'source_verification':original,'coverage':coverage,'authority':'TS retains desktop, scheduler and model process authority. Rust shadow adapters only.','pending_gates':limitations,'rollback':'Discard private fixtures/settings; retain original runtime material and immutable prior checkpoints.'})
        print(json.dumps({'phase':'P06','status':'passed','contracts':sum(counts.values()),'model_actions':0}))
    except Exception as exc:
        write_json(completion,{'phase':'P06','status':'failed','error':str(exc),'commands':commands});raise
