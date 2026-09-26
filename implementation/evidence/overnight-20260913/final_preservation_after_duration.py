"""Run the unchanged preservation audit after a completed, possibly failed run.

This does not finalize quality, alter a duration result, or claim acceptance.
Run before merging followups so the original source bindings still match.
"""
import datetime,json,os,sys
from pathlib import Path
ROOT=Path('/home/alex/Projects/AG1');os.chdir(ROOT);sys.path.insert(0,str(ROOT/'tools'))
from overnight_stress import Investigation
from release_gate import artifact,fingerprints,ref
from resources import identity,same_process
from workload_checks import artifacts_intact
from faults import retain
run=Path(sys.argv[1]).resolve()
if run.parent != ROOT/'.local/p14-long-horizon':raise RuntimeError('Expected a direct disposable duration destination')
result=json.loads((run/'duration-result.json').read_text())
started=json.loads((run/'started.json').read_text())
scope=json.loads((run/'investigation-scope.json').read_text())
if fingerprints()!=scope['source_fingerprints']:raise RuntimeError('Frozen source changed before post-duration audit')
for value in started['bound_inputs']:artifact(value)
known={}
for sample in result['samples']:
 for process in sample['processes']:known[(process['pid'],process['start_ticks'])]=process
alive=[]
for process in known.values():
 current=identity(process['pid'])
 if same_process(process,current) and current['state']!='Z':alive.append(process)
if alive:raise RuntimeError('Observed test processes are still alive; review owned cleanup first: '+str([p['pid'] for p in alive]))
out=run/'post-duration-preservation';out.mkdir(mode=0o700)
retain(out/'started.json',{'utc':datetime.datetime.now(datetime.timezone.utc).isoformat(),'duration_result':ref(run/'duration-result.json'),'duration_status_unchanged':result['status'],'observed_duration_hours':(result['end_monotonic_ns']-result['start_monotonic_ns'])/3.6e12,'scope':'Post-duration byte/authority preservation only. Duration, memory and quality failures remain unchanged. No release acceptance.'})
owner=Investigation.__new__(Investigation);owner.out=out;owner.config=run/'native/config.json';owner.commands=[]
owner.cutover=artifact(next(v for v in started['bound_inputs'] if Path(v['path']).name=='ag1-cutover'))
owner.final_preservation()
retained=artifacts_intact(run/'native/state',json.loads((run/'native/seed.json').read_text()))
retain(out/'result.json',{'status':'preservation-checked','duration_status_unchanged':result['status'],'release_acceptance':False,'observed_processes_closed':len(known),'original_artifacts':retained,'preservation':ref(out/'final-preservation.json'),'commands':owner.commands})
print('Post-duration preservation complete; duration/memory/quality assessment unchanged.',flush=True)
