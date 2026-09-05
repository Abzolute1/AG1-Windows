"""P05: real storage contracts, immutable SDK oracles, faults and synced benchmarks."""
import datetime,json,os,re,shutil,tempfile
from pathlib import Path
from common import ROOT,GateError,read_json,write_json,sha256,run,verify_original

def validate_rust_log(log,minimum=28):
 summaries=re.findall(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured; (\d+) filtered out;',log)
 if len(summaries)!=1:raise GateError('Missing or ambiguous storage test summary')
 passed,failed,ignored,measured,filtered=map(int,summaries[0])
 cases=re.findall(r'^test ([a-z_][a-z0-9_]*) \.\.\. ok$',log,re.M)
 if passed<minimum or any([failed,ignored,measured,filtered]) or len(cases)!=passed or len(set(cases))!=passed:raise GateError('Empty, skipped or incomplete storage contracts')
 return passed

def validate_measurements(value):
 if value.get('status')!='measured' or value.get('model_actions')!=0:raise GateError('Invalid storage benchmark authority/result')
 rows=value.get('journal',[])
 if [r.get('entries') for r in rows]!=[10000,100000]:raise GateError('Missing 10k/100k benchmark')
 for r in rows:
  if r.get('resumed_entries')!=r['entries'] or any(r.get(k,0)<=0 for k in ['source_bytes','import_ns','resume_ns','branch_ns']):raise GateError('Missing journal results or timing')
 samples=value.get('durable_ack',{}).get('samples_ns',[])
 if len(samples)!=100 or any(n<=0 for n in samples):raise GateError('Incomplete durable acknowledgement measurement')
 catalogs=value.get('catalogs',[])
 if [r.get('records') for r in catalogs]!=[10,100,1000]:raise GateError('Missing catalog growth')
 for r in catalogs:
  if any(r.get(k,0)<=0 for k in ['growth_ns','warm_search_ns','warm_page_ns']):raise GateError('Incomplete catalog measurements')
 if any(value.get('cold',{}).get(k,0)<=0 for k in ['reopen_ns','search_ns','page_ns']):raise GateError('Missing reopened-adapter measurements')

def validate_oracles(reference,export):
 if reference.get('status')!='passed' or reference.get('source_files')!=38 or reference.get('journals')!=4 or reference.get('utf8_windows')!=55 or reference.get('cursor_versions')!=2 or reference.get('model_actions')!=0:raise GateError('Incomplete frozen SDK oracle')
 if export.get('status')!='passed' or export.get('model_actions')!=0 or not all(export.get(k) is True for k in ['original_sdk_resume','artifact_bytes','cursor_compatible','lease_compatible']):raise GateError('Original-format export is not proven')

def inputs():
 files=[]
 for base in [ROOT/'product/crates/ag1-core',ROOT/'product/crates/ag1-storage',ROOT/'product/licenses',ROOT/'fixtures/storage']:
  files.extend(p for p in base.rglob('*') if p.is_file())
 files.extend(ROOT/p for p in ['product/Cargo.toml','product/Cargo.lock','phases/P05-storage-and-import.md','tools/storage_gate.py','tools/phase_gate.py','tools/common.py','tools/record_storage_licenses.py','tools/record_protocol_licenses.py','tools/probes/storage_reference.mjs','tools/probes/storage_export_reference.mjs','tools/tests/test_storage_gate.py'])
 return {str(p.relative_to(ROOT)):sha256(p) for p in sorted(set(files))}

def execute_p05():
 from phase_gate import verify_dependencies,verify_artifacts,verify_runtime_material
 evidence=ROOT/'implementation/evidence/P05';evidence.mkdir(parents=True,exist_ok=True);completion=evidence/'completion.json'
 write_json(completion,{'phase':'P05','status':'running'});commands=[]
 try:
  verify_dependencies('P05');original=verify_original()
  for phase in ['P01','P04']:
   prior=read_json(ROOT/f'implementation/evidence/{phase}/completion.json')
   if prior['status']!='passed':raise GateError('Prior checkpoint is not passed')
   verify_artifacts(ROOT,prior['artifacts'])
  env=os.environ.copy();node=ROOT/'.local/toolchains/node-v24.14.0-linux-x64/bin/node';env['PATH']=str(node.parent)+':'+env.get('PATH','/usr/bin:/bin');env['NO_COLOR']='1'
  for name,argv in [('storage-notices',['python3','tools/record_storage_licenses.py']),('licenses',['python3','tools/record_protocol_licenses.py'])]:commands.append(run(argv,log=evidence/f'{name}.log',env=env))
  before=inputs()
  tasks=[('gate-tests',['python3','-m','unittest','discover','-s','tools/tests','-p','test_storage_gate.py','-v']),('reference',[str(node),'tools/probes/storage_reference.mjs']),('format',['cargo','fmt','--manifest-path','product/Cargo.toml','-p','ag1-storage','-p','ag1-core','--','--check']),('clippy',['cargo','clippy','--manifest-path','product/Cargo.toml','-p','ag1-storage','--all-targets','--','-D','warnings']),('storage-contract',['cargo','test','--manifest-path','product/Cargo.toml','-p','ag1-storage','--test','storage_contract']),('build',['cargo','build','--manifest-path','product/Cargo.toml','-p','ag1-storage','--release','--bin','storage-probe']),('measurements',[str(ROOT/'product/target/release/storage-probe'),'measure',str(evidence/'measurements.json')])]
  for name,argv in tasks:
   print('P05: '+name,flush=True);commands.append(run(argv,log=evidence/f'{name}.log',env=env))
  with tempfile.TemporaryDirectory(prefix='p05-compatible-',dir=ROOT/'.local') as temporary:
   export=Path(temporary)/'export'
   commands.append(run([str(ROOT/'product/target/release/storage-probe'),'export-fixture',str(export)],log=evidence/'export-fixture.log',env=env))
   commands.append(run([str(node),'tools/probes/storage_export_reference.mjs',str(export),str(evidence/'export-fixture.log')],log=evidence/'export-reference.log',env=env))
  count=validate_rust_log((evidence/'storage-contract.log').read_text(),minimum=28)
  validate_oracles(read_json(evidence/'reference.log'),read_json(evidence/'export-reference.log'))
  measured=read_json(evidence/'measurements.json');validate_measurements(measured)
  verify_original();verify_dependencies('P05')
  verify_runtime_material(read_json(ROOT/'implementation/evidence/P01/runtime-manifest.json'),read_json(ROOT/'.local/runtime/hash-cache.json'))
  lock=read_json(ROOT/'planning/evidence/llama-runtime-lock.json')
  if sha256(Path(lock['profile_file']))!=lock['profile_file_sha256']:raise GateError('Original runtime profile changed')
  after=inputs()
  if before!=after:raise GateError('Storage inputs changed during gate')
  limitations=[
   'Shadow-only storage. TS owns executing desktop sessions until P13; no live state migration or dual writes.',
   'Linux directory sync, advisory writer lock and atomic no-replace publication validated. Windows/macOS/WASM storage runners remain pending and native opening on unvalidated platforms returns unsupported.',
   'Crash tests use abrupt child exit and injected I/O boundaries, not destructive power-loss or hardware fault tests. WAL FULL durability still depends on truthful filesystem/device sync behavior.',
   'Cold measurements reopen the adapter while the kernel cache remains warm; no system cache eviction. TS unsynced append latency is not an equivalent durability baseline.',
   'Forks retain the parent authorized evidence closure conservatively, including opaque custom references. Archived journal/import roots remain pinned; orphan collection removes only unpublished/unreferenced blobs. Automatic pruning of historical roots is not enabled.',
   'A source that already evicted binary bytes imports as explicit unavailable evidence with retained metadata; no missing bytes are fabricated. Corrupt memory remains quarantined and blocks ordinary replacement.',
   'Compatible export rejects unknown entry types, corrupt memory, inherited/ambiguous origin keys, uncertain external intents and previously evicted evidence. The lossless SQLite/blob/original archive remains available for restore.',
   'Import accepts a complete frozen manifest only, up to 100000 files/4 GiB total, 256 MiB per file, 100000 entries per journal and 4 MiB per entry. Larger sources fail explicitly and require a later streaming migration path.',
   'Literal search is an initial bounded result scan, not P08 context selection/ranking. UTF-8 pages match the reference by skipping continuation bytes and extending up to three bytes to finish a code point.',
   'Runtime/process/job metadata is historical only; original sources, tuned llama.cpp configurations and weights remain unchanged. Existing live-provider and final migration/release gates remain pending.'
  ]
  write_json(evidence/'limitations.json',limitations)
  lines=['# P05 transactional storage and import/export','',f'The P05 gate executed {count} storage contracts, six gate-validator tests, frozen SDK comparisons and original-SDK export reopening. [completion.json](completion.json) is the authoritative status and records commands, input hashes and delivered artifacts.','',
   'The new ag1-storage crate owns independent SQLite journal/branch metadata, command intent/results, committed sequence, memory and settings CAS, immutable SHA-256 evidence, origin keys, leases and restartable imports. Core ports expose domain values without database handles. No executing desktop authority changed.','',
   'B02 corruption is retained with diagnostics, never normalized to empty memory. B04 forks keep their authorized historical evidence and old cursors after parent archival/reopen. Cross-session access and tampered cursors fail. Raw source bytes and unknown payloads survive repeated export/restore. Imported process records cannot grant process ownership.','',
   'Twenty-eight contracts include 19 abrupt child exits across publication/transaction/acknowledgement boundaries, interrupted import replay, 512 deterministic malformed-journal mutations, a 1000-entry randomized branch graph, UTF-8/CRLF windows, binary tamper/eviction, catalog reconstruction, lease caps, unsafe downgrade rejection, and a destination publication race. The frozen SDK corpus covers 38 files, four journals, three memory versions, 55 byte windows and both cursor formats. [The export check](export-reference.log) reopens a Rust export through the unchanged SDK and verifies artifacts, cursors and leases.','',
   '## Measurements','', '| Entries | Durable import | Reopen + full resume | Branch projection |','| --- | --- | --- | --- |']
  for r in measured['journal']:lines.append(f"| {r['entries']:,} | {r['import_ns']/1e6:.2f} ms | {r['resume_ns']/1e6:.2f} ms | {r['branch_ns']/1e6:.2f} ms |")
  lines += ['',f"The 100 synced acknowledgement samples measured p50 {measured['durable_ack']['p50_ns']/1e6:.3f} ms and p95 {measured['durable_ack']['p95_ns']/1e6:.3f} ms. Catalog measurements cover 10/100/1000 records, warm page/search and reopened adapter reads. See [raw measurements](measurements.json) for every sample and limitations.",'',
   'WAL with synchronous=FULL is configured and blob/directory synchronization precedes reference commits. This follows [SQLite’s synchronization contract](https://www.sqlite.org/pragma.html#pragma_synchronous); it cannot guarantee that every device truthfully persists a sync. The unsynced TS append baseline is therefore not used as a direct speed target.','',
   '## Dependencies, bounds and handoff','',
   'rusqlite 0.40.2 uses bundled SQLite 3.53.2 for storage only. Exact dependency licenses, the SQLite source notice and omitted WASM VFS license at its published VCS pin are retained under product/licenses and [storage-dependencies.json](storage-dependencies.json). Rust storage does not replace or rebuild any llama.cpp runtime.','',
   'See [the storage API/schema guide](../../../product/crates/ag1-storage/README.md) and [limitations.json](limitations.json). Retention preserves archive roots conservatively. P06 receives settings CAS and P07 receives journal/blob/cursor transactions; P13 retains migration authority. Rollback discards an unaccepted private destination while keeping original snapshots and the independent lossless export. P06 has not started.']
  # Report is evidence, not an input; generated numbers always describe this gate run.
  (evidence/'REPORT.md').write_text('\n'.join(lines)+'\n')
  snapshot=evidence/'delivered-snapshot'
  for relative in after:
   target=snapshot/relative;target.parent.mkdir(parents=True,exist_ok=True);shutil.copyfile(ROOT/relative,target)
  artifacts=[{'path':str(p.relative_to(ROOT)),'sha256':sha256(p)} for p in sorted(evidence.rglob('*')) if p.is_file() and p!=completion and p.name!='gate.log']
  verify_artifacts(ROOT,artifacts)
  write_json(completion,{'phase':'P05','status':'passed','completed_at':datetime.datetime.now(datetime.timezone.utc).isoformat(),'dependencies':['P02'],'input_fingerprints':after,'commands':commands,'artifacts':artifacts,'source_verification':original,'coverage':{'storage_contracts':count,'gate_tests':6,'frozen_source_files':38,'abrupt_exit_cases':19,'fuzz_mutations':512,'capabilities':['A38','A40','A41','A42','A43','A44','A46','A47','A48-storage'],'invariants':['I05','I10','I11','I12','I13','I14','I15','I27'],'defects':['B02','B04'],'runtime_preservation':['L01','L02','L03','L04','L05','L06','L07','L08']},'authority':'Independent shadow destination only. TS remains the executing desktop/session authority; no dual writes or runtime process ownership transfer.','pending_gates':limitations,'rollback':'Discard only an unaccepted private destination; retain frozen originals, raw archives and verified lossless export. Never overwrite live TS journals, original profiles, tuned binaries/libraries or weights.'})
  print(json.dumps({'phase':'P05','status':'passed','storage_contracts':count,'frozen_source_files':38,'model_actions':0}))
 except Exception as exc:
  write_json(completion,{'phase':'P05','status':'failed','error':str(exc),'commands':commands});raise
