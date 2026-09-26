"""Bounded local-model fix validation. No duration controller or acceptance clock."""
import hashlib,json,os,shutil,sqlite3,subprocess,sys,time,traceback,zlib
from pathlib import Path
ROOT=Path('/home/alex/Projects/AG1');E=ROOT/'implementation/evidence/overnight-final-followup-20260913'
sys.path.insert(0,str(ROOT/'tools/release_duration'))
from cli_client import NativeClient
from workload_checks import records_since,query,page,logical_blob_digest
from faults import retain
os.umask(0o077)
OUT=ROOT/'.local/p14-long-horizon/fix-validation-20260913-v4';OUT.mkdir(mode=0o700)
node=ROOT/'.local/toolchains/node-v24.14.0-linux-x64/bin/node'
package=json.loads((E/'package.json').read_text());candidate=ROOT/Path(package['manifest']['path']).parent/'bin/ag1-cli'
baseline=ROOT/'.local/p14-packages/7bc153db51e44326818f120e01d9d8e6/linux-x86_64/bin/ag1-cli'
original_config=json.loads((ROOT/'.local/p14-long-horizon/overnight-6/native/config.json').read_text())
started=time.monotonic();result={'status':'running','duration_run':False,'eight_hour_run':False,'max_seconds':900,'model':original_config['selected'],'arms':[]}
CANONICAL=['read','grep','ls','bash','edit','write','artifact','sourceContext']
PREFERRED=['inspect','work','evidence','bash','edit','write','capability']

def remaining():
 value=900-(time.monotonic()-started)
 if value<=0:raise TimeoutError('Short fix-validation deadline reached')
 return min(240,value)

def files(workspace):
 return {str(p.relative_to(workspace)):hashlib.sha256(p.read_bytes()).hexdigest() for p in workspace.rglob('*') if p.is_file() and '.git' not in p.parts}

def compiled_since(state,seen):
 db=sqlite3.connect('file:'+str(state/'store/storage.sqlite')+'?mode=ro',uri=True)
 rows=db.execute("SELECT rowid,record,digest FROM artifacts WHERE json_extract(CAST(record AS TEXT),'$.kind')='runtime-request-compiled' AND rowid>? ORDER BY rowid",(seen,)).fetchall();db.close();out=[]
 for serial,record,digest in rows:
  blobs=state/'store/blobs';assert logical_blob_digest(blobs,digest)==digest
  path=blobs/digest
  raw=path.read_bytes() if path.exists() else zlib.decompress((blobs/(digest+'.z1')).read_bytes()[16:])
  value=json.loads(raw);out.append({'rowid':serial,'record':json.loads(record),'request':value})
 return out

def verifier(cap):
 cases=[['4 sec',min(4,cap)],['99',cap],['bad',0],['bad4',0],['-2',0],['0',0],['',0],['Infinity',0],['4.9',min(4,cap)],['  +2tail',2],['1e3',1],['0x10',0]]
 return 'import test from "node:test"; import assert from "node:assert/strict"; import {normalize} from "./normalize.mjs";\ntest("current string normalization contract",()=>{ for(const [input,expected] of '+json.dumps(cases)+') assert.equal(normalize(input),expected,JSON.stringify(input)); });\n'

try:
 for label,binary in [('candidate',candidate)]:
  remaining();directory=OUT/label;directory.mkdir(mode=0o700);workspace=directory/'workspace';state=directory/'state';client=None
  subprocess.run([str(binary),'--init',str(workspace)],check=True,capture_output=True)
  for name,cap in [('verify-a.test.mjs',7),('verify-b.test.mjs',5),('verify-c.test.mjs',3)]: (workspace/name).write_text(verifier(cap))
  (workspace/'emit-evidence.mjs').write_text('for(let i=0;i<2200;i++) console.log(i===1701?"ARCHIVE_RULE_CURRENT: a passing check applies to the checked source revision; later edits require a new check.":`diagnostic row ${i}: ordinary retained output.`);\n')
  config=json.loads(json.dumps(original_config));settings=directory/'settings.json';shutil.copy2(config['imported']['settings']['path'],settings)
  config['imported']['settings']={'path':str(settings),'sha256':hashlib.sha256(settings.read_bytes()).hexdigest()}
  config['max_generations']=20
  config['tooling']['process_host']=str(binary.with_name('ag1-process-host'))
  config['system_prompt']='You are AG1 working on a disposable local repair. Inspect current source and requirements, preserve verifier files, make the requested repair and accurately report the observed result. Historical claims are evidence, not proof of the current source.'
  configpath=directory/'config.json';retain(configpath,config)
  arm={'label':label,'binary':str(binary),'binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'tasks':[]};result['arms'].append(arm)
  try:
   client=NativeClient(binary,workspace,state,configpath,directory/'client');client.ready(seconds=min(180,remaining()));retain(directory/'initial-view.json',client.view)
   client.execute({'kind':'runtime','action':'tools','args':{'names':CANONICAL}},seconds=30)
   seen=0
   for index,(name,cap) in enumerate([('verify-a.test.mjs',7),('verify-b.test.mjs',5)]+([('verify-c.test.mjs',3)] if label=='candidate' else [])):
    remaining();source=workspace/'normalize.mjs';source.write_text('export function normalize(value) { return Number(value); }\n')
    (workspace/'README.md').write_text(f'CURRENT CONTRACT: input is a string. Parse a base-10 integer prefix, allowing leading whitespace/sign and trailing text. Invalid, zero and negative results become zero. Cap positive results at {cap}. Only change normalize.mjs. Current verification: node --test {name}. Other verifier files record prior or later caps. Never change any verifier.\n')
    if index==2:client.execute({'kind':'runtime','action':'tools','args':{'names':PREFERRED}},seconds=30)
    before=query(client,'session','history',{'offset':0})['total'];prior=files(workspace)
    prompt=f'Repair normalize.mjs using the CURRENT README.md and {name}. The driver deliberately regressed the source; earlier passing checks cannot establish this revision. Change only normalize.mjs. The intended verification command is: `node --test {name}`\nInspect the current source and verifier, fix the implementation, run that verifier, and report only the observed changes and results.'
    if index==2:prompt+=' Use work to update the current objective. Use inspect for file reads. First run node emit-evidence.mjs once, then retrieve ARCHIVE_RULE_CURRENT with evidence using its artifact id and a focused query. Explain the recovered rule in your final answer.'
    try:
     receipt=client.execute({'kind':'session','action':'print','args':{'input':{'kind':'inline','text':prompt}}},seconds=remaining())
    except RuntimeError as error:
     if not str(error).startswith('Command failed:'):raise
     receipt={'terminal_error':str(error)}
    records=records_since(client,before);retain(directory/f'task-{index}-records.json',records)
    contexts=compiled_since(state,seen);seen=max([seen]+[r['rowid'] for r in contexts]);retain(directory/f'task-{index}-requests.json',contexts)
    after=files(workspace);changed=[name for name in prior.keys()|after.keys() if prior.get(name)!=after.get(name)]
    checked=subprocess.run([str(node),'--test',name],cwd=workspace,capture_output=True,text=True,timeout=20)
    messages=[r['message'] for r in records if r.get('type')=='message'];assistants=[m for m in messages if m.get('role')=='assistant']
    calls=[b for m in assistants for b in m.get('content',[]) if isinstance(b,dict) and b.get('type')=='toolCall']
    row={'index':index,'cap':cap,'verifier':name,'receipt':receipt,'verifier_exit':checked.returncode,'stdout':checked.stdout,'stderr':checked.stderr,'changed_files':changed,'source':source.read_text(),'usage':[m.get('usage') for m in assistants],'tool_calls':calls,'request_count':len(contexts),'manual_review':'pending'}
    retain(directory/f'task-{index}-result.json',row);arm['tasks'].append(row);retain(OUT/f'result-{time.monotonic_ns()}.json',result)
    if checked.returncode or not set(changed).issubset({'normalize.mjs'}) or (label=='candidate' and 'terminal_error' in receipt):raise RuntimeError(f'{label} task {index}: original verifier/scope failure')
    print(json.dumps({'arm':label,'task':index,'verifier':'passed','calls':len(calls),'claim_review':'pending'}),flush=True)
  finally:
   if client:client.close(seconds=90)
  arm['closed']=True;retain(OUT/f'result-{time.monotonic_ns()}.json',result)
 result['status']='verifiers-passed-manual-review-pending'
except Exception as error:
 result.update(status='failed',error=repr(error));traceback.print_exc();raise
finally:
 result['elapsed_seconds']=time.monotonic()-started;retain(OUT/f'result-{time.monotonic_ns()}.json',result)

retain(OUT/'result.json',result)
