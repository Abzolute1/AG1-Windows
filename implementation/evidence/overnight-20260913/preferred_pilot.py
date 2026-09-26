"""Fresh-session preferred-tool live probe, separate from duration acceptance."""
import hashlib,json,os,shutil,subprocess,sys,time,traceback
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'tools/release_duration'))
from cli_client import NativeClient
from workload_checks import records_since,query,page
from faults import retain
out=ROOT/'.local/p14-long-horizon/overnight-preferred-pilot-2'
if out.exists():raise RuntimeError('Fresh disposable destination required')
os.umask(0o077);out.mkdir();workspace=out/'workspace';state=out/'state'
subprocess.run(['/opt/ag1/releases/296fc7851ded4281bc649e350803e894/bin/ag1-cli','--init',str(workspace)],check=True)
source=workspace/'normalize.mjs';source.write_text('export function normalize(value) { return Number(value); }\n')
verifier=workspace/'verify.mjs'
def verification(cap):
    return f'import assert from "node:assert/strict"; import {{normalize}} from "./normalize.mjs"; assert.equal(normalize("4 sec"),4); assert.equal(normalize("99"),{cap}); assert.equal(normalize("bad"),0); assert.equal(normalize("-2"),0); console.log("normalize cap {cap} passed");\n'
verifier.write_text(verification(7))
(workspace/'README.md').write_text('normalize: parse a base-10 integer prefix; invalid or nonpositive becomes zero; cap positive values at seven. Only change normalize.mjs. Never edit the verifier.\n')
(workspace/'emit-evidence.mjs').write_text('for(let i=0;i<3500;i++) console.log(i === 2861 ? "ARCHIVE_RULE_CURRENT: receipt records are evidence of the recorded revision, not proof that subsequently edited source is correct." : `catalog row ${i}: stable diagnostic material without an authoritative rule.`);\n')
for argv in [['git','init','-q'],['git','config','user.name','AG1 local probe'],['git','config','user.email','probe@localhost'],['git','add','.'],['git','commit','-qm','Disposable preferred tool fixture']]:subprocess.run(argv,cwd=workspace,check=True)
config=json.loads((ROOT/'.local/p14-long-horizon/overnight-6/native/config.json').read_text())
settings=out/'settings.json';shutil.copy2(config['imported']['settings']['path'],settings)
config['imported']['settings']={'path':str(settings),'sha256':hashlib.sha256(settings.read_bytes()).hexdigest()}
config['system_prompt']='You are AG1 in a disposable local coding exercise. Use the native preferred tools, inspect current evidence before edits, preserve verifier files, and accurately report only observed results.'
configpath=out/'config.json';retain(configpath,config)
node=ROOT/'.local/toolchains/node-v24.14.0-linux-x64/bin/node'
client=None;result={'status':'running','fixture':False,'duration_run':False,'release_acceptance':False,'model':config['selected'],'tasks':[]}
try:
    client=NativeClient(Path('/opt/ag1/releases/296fc7851ded4281bc649e350803e894/bin/ag1-cli'),workspace,state,configpath,out/'client')
    client.ready();retain(out/'initial-view.json',client.view)
    prompts=[
      'Use work to reset the current objective to "Repair normalize using the current local contract". Inspect README.md, normalize.mjs and verify.mjs with the preferred inspection tool. Repair only normalize.mjs, and run node verify.mjs. Also use capability list and capability schema for repo to inspect the advanced interface. Report observed results; do not change the verifier.',
      'Exercise retained evidence recovery: run node emit-evidence.mjs exactly once, then retrieve ARCHIVE_RULE_CURRENT using evidence even if a preview happens to show it. Retain that returned artifact with retainEvidence for verificationScope "preferred-pilot" and reason "checking source revision semantics". Record the recovered rule in work as a decision, then releaseEvidence explicitly because this diagnostic check is over. Use capability run on artifact action leases to confirm the lease is released. Do not change any source files.',
      'The driver has now changed the requirement and deliberately regressed normalize.mjs. Inspect the CURRENT README.md, verifier and source again. Earlier success is stale: cap positive values at FIVE now. Update the work objective, repair only normalize.mjs, and run node verify.mjs. Explain which current result supports completion.'
    ]
    for index,prompt in enumerate(prompts):
        if index==2:
            source.write_text('export function normalize(value) { return Number(value); }\n')
            verifier.write_text(verification(5))
            (workspace/'README.md').write_text('normalize: parse a base-10 integer prefix; invalid or nonpositive becomes zero; cap positive values at five. Only change normalize.mjs. Never edit the verifier.\n')
        before=query(client,'session','history',{'offset':0})['total']
        expected=verifier.read_bytes()
        before_files={str(p.relative_to(workspace)):hashlib.sha256(p.read_bytes()).hexdigest() for p in workspace.rglob('*') if p.is_file() and '.git' not in p.parts}
        receipt=client.execute({'kind':'prompt','input':{'kind':'inline','text':prompt}},seconds=1200)
        after_files={str(p.relative_to(workspace)):hashlib.sha256(p.read_bytes()).hexdigest() for p in workspace.rglob('*') if p.is_file() and '.git' not in p.parts}
        changed=[p for p in before_files.keys() | after_files.keys() if before_files.get(p)!=after_files.get(p)]
        retain(out/f'task-{index}-file-custody.json',{'before':before_files,'after':after_files,'changed':changed})
        records=records_since(client,before);retain(out/f'task-{index}-records.json',records)
        context=page(client,'runtime','detail_page',{'section':'context'});retain(out/f'task-{index}-context.json',context)
        activity=page(client,'runtime','detail_page',{'section':'activity'});retain(out/f'task-{index}-activity.json',activity)
        checked=subprocess.run([str(node),str(verifier)],cwd=workspace,capture_output=True,text=True,timeout=30)
        row={'index':index,'receipt':receipt,'verifier_exit':checked.returncode,'stdout':checked.stdout,'stderr':checked.stderr,'verifier_unchanged':verifier.read_bytes()==expected,'changed_files':changed,'claim_review_pending':True}
        retain(out/f'task-{index}-result.json',row);result['tasks'].append(row)
        if checked.returncode or verifier.read_bytes()!=expected or not set(changed).issubset({'normalize.mjs'}):raise RuntimeError('Original verifier failed or changed')
        print(json.dumps({'task':index,'status':'passed-verifier','claim_review_pending':True}),flush=True)
    result['status']='passed-verifiers-review-pending'
except Exception as error:
    result.update(status='failed',error=repr(error));traceback.print_exc();raise
finally:
    if client:
        try:client.close()
        except Exception as error:result.update(status='failed',cleanup_error=repr(error))
    retain(out/'result.json',result)
