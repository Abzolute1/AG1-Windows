import hashlib,json,os,subprocess,sys,time
from pathlib import Path
ROOT=Path('/home/alex/Projects/AG1');E=ROOT/'implementation/evidence/overnight-final-followup-20260913';D=ROOT/'.local/p14-long-horizon/fix-validation-20260913-v4/candidate';OUT=ROOT/'.local/p14-long-horizon/fix-validation-20260913-work';os.umask(0o077);OUT.mkdir(mode=0o700)
sys.path.insert(0,str(ROOT/'tools/release_duration'))
from cli_client import NativeClient
from workload_checks import records_since,query
from faults import retain
p=json.loads((E/'package.json').read_text());binary=ROOT/Path(p['manifest']['path']).parent/'bin/ag1-cli';config=json.loads((D/'config.json').read_text());config['tooling']['process_host']=str(binary.with_name('ag1-process-host'));config['max_generations']=8;retain(OUT/'config.json',config)
source=D/'workspace/normalize.mjs';before_source=hashlib.sha256(source.read_bytes()).hexdigest();client=None;start=time.monotonic()
try:
 client=NativeClient(binary,D/'workspace',D/'state',OUT/'config.json',OUT/'client');client.ready(seconds=180);before=query(client,'session','history',{'offset':0})['total']
 prompt='Finish the remaining task-state action from the previous repair. Use work to change the current objective to "Repair normalize.mjs against README.md and verify-c.test.mjs" and nextAction to "Review verified cap-3 repair". Retrieve ARCHIVE_RULE_CURRENT from evidence artifact art_6d14bafd630c57ee6d048303f2581e87 with that id and a focused query. Report only actions that actually succeeded. The source repair and verifier have already passed; no source edits or repeated verifier are needed.'
 receipt=client.execute({'kind':'prompt','input':{'kind':'inline','text':prompt}},seconds=240);records=records_since(client,before);retain(OUT/'records.json',records)
 assert hashlib.sha256(source.read_bytes()).hexdigest()==before_source
 retain(OUT/'result.json',{'receipt':receipt,'source_unchanged':True,'elapsed_seconds':time.monotonic()-start,'manual_review':'pending','binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'duration_run':False})
finally:
 if client:client.close(seconds=90)
