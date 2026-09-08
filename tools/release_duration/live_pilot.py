"""Short real packaged recovery pilot; explicitly excluded from duration acceptance."""
import argparse,json,time,traceback
from pathlib import Path
from run_duration import Run
from faults import retain
from release_gate import ref,input_digest


def main(args):
    args.hours=8
    r=Run(args)
    r.started=time.monotonic_ns();r.sampler.start()
    retain(r.out/'pilot-started.json',{'fixture':False,'duration_run':False,'release_acceptance':False,'start_monotonic_ns':r.started,'input_sha256':input_digest(),'package_sha256':r.package_sha,'bound_inputs':r.bound,'scope':'Six original repairs through packaged CLI and instrumented real WebKit, with real owned model/host exits and disposable archive corruption recovery.'})
    result={'status':'running','fixture':False,'duration_run':False,'release_acceptance':False}
    try:
        r.activity('admission',lambda d:r.open())
        r.activity('resume',lambda d:r.remember_scenario('resume',d,r.control('resume',{'session':r.seed['session']})))
        for index in range(6):
            if index==1:
                injection=r.activity('model-crash',r.model_crash)
            if index==2:r.activity('branch',r.branch_fork)
            r.task(index,through_renderer=index==3)
            if index==1:
                path=r.out/'model-recovery.json';retain(path,{'status':'passed','injection':injection,'successful_next_task':r.tasks[-1]})
                r.scenarios.append({'scenario':'model-crash','executed':True,'passed':True,'evidence':[ref(path)]})
                r.activity('host-crash',r.host_crash)
            if index==2:r.activity('resume',lambda d:r.remember_scenario('resume',d,r.control('resume',{'session':r.seed['session']})))
            if index==4:r.activity('corrupt-state-recovery',r.corrupt)
        r.evidence_bound();r.sampler.sample();r.close();r.sampler.close()
        r.run_command([str(r.cutover),'audit-cutover','/home/alex/Projects/Cruzible'],r.out/'final-preservation.json',1800)
        r.evidence_bound()
        result.update(status='passed',activities=r.activities,tasks=r.tasks,scenarios=r.scenarios,commands=r.commands,allocator_samples=r.sampler.samples,claim_review_pending=True)
    except Exception as error:
        result.update(status='failed',error=repr(error));traceback.print_exc();raise
    finally:
        if r.client is not None:
            try:r.close()
            except Exception as error:result.update(status='failed',cleanup_error=repr(error))
        if not r.sampler.stopped.is_set():
            try:r.sampler.close()
            except Exception as error:result.update(status='failed',sampler_error=repr(error))
        result['finished_monotonic_ns']=time.monotonic_ns();retain(r.out/'pilot-result.json',result)
    if result['status']!='passed':raise RuntimeError('Pilot cleanup failed')

if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('output',type=Path);p.add_argument('--preflight',type=Path,required=True);p.add_argument('--heap-library',type=Path,required=True)
    main(p.parse_args())
