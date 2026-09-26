"""Repeated original repairs with actual monotonic duration and native fault recovery."""
import argparse
import copy
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import time
import traceback
from types import SimpleNamespace
from cli_client import NativeClient
from resources import Sampler,descendants,identity,same_process
from faults import retain,digest,crash_model,crash_idle_host
from workload_checks import query,page,history,introduce,verify,artifacts_intact,git
import renderer
from memory_analysis import analyze as analyze_memory
ROOT=Path.cwd();sys.path.insert(0,str(ROOT/'tools'))
from release_long_horizon import native_input
from release_gate import fingerprints,input_digest,ref,artifact
from common import verify_original
from review_checks import reject_failed_reviews
TOOLS=['read','grep','ls','bash','edit','write','artifact','sourceContext']
POLICY={'max_idle_seconds':120,'max_activity_seconds':1800,'minimum_tasks_per_hour':2}

class Run:
    allow_semantic_continuation = False
    def __init__(self,args):
        os.umask(0o077)
        self.args=args;self.out=args.output.absolute();base=ROOT/'.local/p14-long-horizon'
        if self.out.exists() or self.out.parent.resolve()!=base.resolve():raise RuntimeError('New direct P14 long-horizon destination required')
        self.out.mkdir(mode=0o700)
        self.client=None;self.client_logs=[];self.client_serial=0;self.serial=0;self.activities=[];self.scenarios=[];self.tasks=[];self.commands=[];self.known_children={};self.sampler=None;self.started=None
        self.before=fingerprints();self.pins=verify_original()
        live_result=json.load(open(args.preflight/'finished.json'));native_result=json.load(open(args.preflight/'native/native-result.json'))
        if any(r['exit_code'] for r in live_result['runs']) or not live_result['inputs_unchanged'] or native_result['status']!='passed':raise RuntimeError('A complete matched live preflight is required')
        preflight_inputs=json.load(open(args.preflight/'started.json'))['input_fingerprints']
        relevant=lambda key:key.startswith(('product/crates/','product/apps/','fixtures/release/')) or key in ('product/Cargo.toml','product/Cargo.lock')
        if {k:v for k,v in preflight_inputs.items() if relevant(k)}!={k:v for k,v in self.before.items() if relevant(k)}:raise RuntimeError('Current product differs from matched live preflight')
        self.ts=args.preflight/'ts';self.seed_source=json.load(open(self.ts/'seed.json'))
        if self.seed_source['fixture'] or not self.seed_source['token_counts_exact'] or not 2000000<=self.seed_source['token_count']<=3000000:raise RuntimeError('Exact live externalized history required')
        # The final candidate and scripts are frozen before duration begins.
        package=json.load(open(ROOT/'implementation/evidence/P14/package.json'))
        archive=artifact(package['archive']);self.package_sha=package['archive']['sha256']
        if package['input_sha256']!=input_digest():raise RuntimeError('Package does not match current release inputs')
        spec=importlib.util.spec_from_file_location('p14_installer',ROOT/'product/release/install.py');installer=importlib.util.module_from_spec(spec);spec.loader.exec_module(installer)
        installed=installer.install(archive,self.package_sha,self.out/'installation');self.binary=installed/'bin/ag1-cli';self.cutover=installed/'bin/ag1-cutover'
        self.live=json.load(open(args.preflight/'input.json'))['live'];self.model=next(m for m in self.live['models'] if m['id']==self.seed_source['model']['id'])
        self.native=self.out/'native';request=native_input(self.ts,self.native,self.live,self.model);value=json.load(open(request));value['prepare_only']=True;request.write_text(json.dumps(value,indent=2)+'\n')
        self.prepare_binary=ROOT/'.local/p14-quality-target/release/examples/release-long-horizon-native'
        self.archive_binary=ROOT/'.local/p14-quality-target/release/examples/release-recovery-checkpoint'
        self.desktop_binary=ROOT/'.local/p14-quality-target/release/ag1-desktop'
        self.heap_library=args.heap_library.absolute()
        self.run_command([str(self.prepare_binary),str(request)],self.out/'prepare.log',120)
        self.workspace=self.native/'workspace';self.state=self.native/'state';self.seed=json.load(open(self.native/'seed.json'));self.specs=json.load(open(self.native/'specs.json'));self.node=value['node']
        self.config=self.native/'config.json';retain(self.config,value['config']);retain(self.state/'host.json',value['config'])
        capture=json.load(open(self.live['capture']['path']));self.expected_plan=next(m['expected'] for m in capture['request']['models'] if m['model_id']==self.model['id'])
        self.bound=[ref(self.binary),ref(self.cutover),ref(self.prepare_binary),ref(self.archive_binary),ref(self.desktop_binary),ref(self.heap_library)]
        self.bound += [ref(p) for p in sorted(Path(__file__).parent.glob('*.py'))]
        if fingerprints()!=self.before:raise RuntimeError('Preparation changed release inputs')
        self.sampler=Sampler(self.out/'memory.jsonl',self.state)

    def run_command(self,argv,log,timeout):
        with log.open('x') as stream:r=subprocess.run(argv,cwd=ROOT,stdout=stream,stderr=subprocess.STDOUT,timeout=timeout)
        self.commands.append({'argv':argv,'exit_code':r.returncode,'log':str(log.relative_to(ROOT))})
        if r.returncode:raise RuntimeError(f'Command failed; see {log}')
        return r

    def evidence_bound(self):
        for value in self.bound:artifact(value)
        if fingerprints()!=self.before or verify_original()!=self.pins:raise RuntimeError('Frozen duration inputs or retained original pins changed')
        return artifacts_intact(self.state,self.seed)

    def activity(self,kind,work):
        reject_failed_reviews(self.out,self.tasks,allow_semantic_continuation=self.allow_semantic_continuation)
        self.serial+=1;name=f'{self.serial:06}-{kind}';directory=self.out/'activities'/name;directory.mkdir(parents=True,mode=0o700)
        begin=time.monotonic_ns()
        if self.activities and begin-self.activities[-1]['finished_monotonic_ns']>POLICY['max_idle_seconds']*1e9:raise RuntimeError('Duration activity gap exceeded frozen budget')
        retain(directory/'started.json',{'id':name,'kind':kind,'started_monotonic_ns':begin})
        offsets={str(p):p.stat().st_size for p in self.out.glob('client-*/protocol.jsonl')}
        try:
            result=work(directory)
            end=time.monotonic_ns()
            if end-begin>POLICY['max_activity_seconds']*1e9:raise RuntimeError('Activity exceeded its bounded duration')
            if self.sampler.error:raise RuntimeError('Memory observation failed: '+self.sampler.error)
            retain(directory/'result.json',result)
            for protocol in sorted(self.out.glob('client-*/protocol.jsonl')):
                with protocol.open('rb') as stream:
                    stream.seek(offsets.get(str(protocol),0));chunk=stream.read()
                if chunk:
                    # Preserve complete protocol records from this activity.
                    chunk=chunk[:chunk.rfind(b'\n')+1]
                    (directory/(protocol.parent.name+'-protocol.jsonl')).write_bytes(chunk)
            evidence=[ref(p) for p in sorted(directory.iterdir()) if p.is_file()]
            receipt={'evidence':evidence,'status':'passed','id':name,'kind':kind,'started_monotonic_ns':begin,'finished_monotonic_ns':end,'result':result}
            retain(directory/'finished.json',receipt)
            row={k:receipt[k] for k in ('id','kind','started_monotonic_ns','finished_monotonic_ns')};row.update(passed=True,finished_receipt=ref(directory/'finished.json'));self.activities.append(row)
            retain(self.out/f'progress-{self.serial:06}.json',{'elapsed_seconds':(end-self.started)/1e9,'completed_tasks':len(self.tasks),'latest':row})
            return result
        except (Exception, KeyboardInterrupt) as error:
            retain(directory/'failed.json',{'status':'failed','id':name,'kind':kind,'started_monotonic_ns':begin,'finished_monotonic_ns':time.monotonic_ns(),'error':repr(error)})
            raise

    def open(self):
        self.client_serial+=1
        self.client=NativeClient(self.binary,self.workspace,self.state,self.config,self.out/f'client-{self.client_serial}',self.heap_library)
        deadline=time.monotonic()+10
        while not (self.client.output/f'heap.{self.client.process.pid}.jsonl').exists():
            if time.monotonic()>deadline or self.client.process.poll() is not None:raise RuntimeError('Native allocator sampler was not installed')
            time.sleep(.05)
        self.client_logs.append(self.client.output)
        self.sampler.bind(self.client);self.client.ready()
        return {'pid':self.client.process.pid,'scope':self.client.view['snapshot']['scope']}

    def close(self,crashed=False):
        if self.client is None:return
        with self.sampler.lock:self.known_children.update(self.sampler.known_children)
        root=identity(self.client.process.pid)
        if root:self.known_children.update({r['pid']:r for r in descendants(root) if r['pid']!=root['pid']})
        try:self.client.close()
        except RuntimeError as error:
            if not crashed or str(error)!='CLI drain failed: -9':raise
        self.commands.append({'argv':self.client.argv,'exit_code':self.client.process.returncode,'log':str((self.client.output/'protocol.jsonl').relative_to(ROOT)),'declared_crash':crashed})
        self.sampler.unbind();self.client=None
        deadline=time.monotonic()+30
        while True:
            leaks=[]
            for pid,original in self.known_children.items():
                observed=identity(pid)
                if same_process(original,observed) and observed['state']!='Z':leaks.append(pid)
            if not leaks:break
            if time.monotonic()>deadline:raise RuntimeError(f'Owned children did not settle: {leaks}')
            time.sleep(.1)
        retain(self.out/f'children-settled-{self.client_serial}.json',{'known_children':list(self.known_children.values()),'leaked_owned_children':0,'monotonic_ns':time.monotonic_ns(),'criterion':'Every descendant identity recorded at minute samples or final tree inspection has exited; PID reuse does not grant identity.'})

    def control(self,action,args):
        return self.client.execute({'kind':'session','action':action,'args':args},seconds=120)

    def task(self,index,through_renderer=False):
        spec=self.specs[index%len(self.specs)]
        def work(directory):
            self.client.execute({'kind':'runtime','action':'tools','args':{'names':TOOLS}})
            introduce(self.workspace,spec,index+1)
            before=query(self.client,'session','history',{'offset':0})['total']
            full_context=None
            if through_renderer:
                self.close()
                result=renderer.run(self.desktop_binary,self.workspace,self.config,self.out/f'renderer-{index+1}',spec['task'],False,self.sampler,self.heap_library,after_task=getattr(self,'renderer_probe',None))
                full_context=result['full_context'];self.open()
                self.scenarios.append({'scenario':'detach-reconnect','executed':True,'passed':True,'evidence':[ref(self.out/f'renderer-{index+1}/renderer-result.json')]})
                receipt=result['terminal']
            else:
                receipt=self.client.execute({'kind':'prompt','input':{'kind':'inline','text':spec['task']}},seconds=1200)
                retain(directory/'command.json',receipt)
            verified=verify(self.client,self.workspace,self.state,self.specs,spec,before,directory,self.node,full_context)
            self.tasks.append({'index':index,'spec':spec['id'],'verification':ref(directory/'verification.json'),'records':ref(directory/'records.json'),'command_receipt':receipt,'claim_review_required':True})
            return verified
        return self.activity('task',work)

    def remember_scenario(self,name,directory,result):
        retain(directory/'scenario.json',result)
        self.scenarios.append({'scenario':name,'executed':True,'passed':True,'evidence':[ref(directory/'scenario.json')]})
        return result

    def branch_fork(self,directory):
        rows=history(self.client);users=[r for r in rows if r.get('role')=='user']
        original=self.client.view['snapshot']['scope']['session']
        fork=self.control('fork',{'entry':users[-1]['id']});retain(directory/'fork.json',fork)
        if self.client.view['snapshot']['scope']['session']==original:raise RuntimeError('Fork did not select a new session')
        self.scenarios.append({'scenario':'fork','executed':True,'passed':True,'evidence':[ref(directory/'fork.json')]})
        # Forking at a user entry drops that task's later assistant messages.
        # Select from the fork itself, and distinguish final answers from visible
        # assistant commentary attached to tool calls.
        fork_rows=history(self.client);examined=[];completed=[]
        for row in reversed(fork_rows):
            if row.get('role')!='assistant':continue
            entry=page(self.client,'session','entry',{'id':row['id']})
            examined.append(entry);message=entry.get('message',{})
            if message.get('stopReason')=='stop' and not any(b.get('type')=='toolCall' for b in message.get('content',[])):
                completed.append(entry)
                if len(completed)==2:break
        retain(directory/'branch-selection.json',{'fork_history':fork_rows,'examined_entries':examined,'completed_ids':[entry['id'] for entry in completed]})
        if len(completed)<2:raise RuntimeError('Fork lacks two completed assistant checkpoints for branch coverage')
        branch=self.control('tree',{'target':completed[1]['id'],'summarize':False});retain(directory/'branch.json',branch)
        return self.remember_scenario('branch',directory,{'fork':ref(directory/'fork.json'),'branch':ref(directory/'branch.json'),'original_session':original})

    def host_crash(self,directory):
        last=self.tasks[-1]['command_receipt']
        if 'envelope' not in last:raise RuntimeError('Host crash must follow a CLI checkpoint receipt')
        before_files={s['sourcePath']:digest(self.workspace/s['sourcePath']) for s in self.specs}
        before_history=query(self.client,'session','history',{'offset':0})['total']
        before_head=git(self.workspace,'rev-parse','HEAD')
        crash_idle_host(self.client,directory);self.close(crashed=True);self.open()
        after_files={s['sourcePath']:digest(self.workspace/s['sourcePath']) for s in self.specs}
        if before_files!=after_files or before_head!=git(self.workspace,'rev-parse','HEAD') or git(self.workspace,'status','--porcelain').strip():raise RuntimeError('Host restart changed the verified source state')
        if query(self.client,'session','history',{'offset':0})['total']!=before_history:raise RuntimeError('Host restart changed the acknowledged journal')
        observations=self.client.view['details']['activity'].get('observations')
        if observations is not None and observations['provider_stops']!=0:raise RuntimeError('Host restart generated an unsolicited model response')
        if self.client.view['snapshot']['status']!='idle' or self.client.view['snapshot']['active_operation'] is not None:raise RuntimeError('Host restart began unrequested work')
        envelope=self.client.send({'kind':'command_result','command':last['envelope']['id']},scope=last['envelope']['scope']);reply=self.client.reply(envelope)
        if reply['data']!=last['terminal']:raise RuntimeError('Completed command receipt changed after crash')
        return self.remember_scenario('host-crash',directory,{'recovered_receipt':reply,'source_files_before_and_after':before_files,'unchanged_journal_entries':before_history,'effects_replayed':False,'criterion':'Exact completed pre-crash command receipt; source worktree remains unchanged and next original repair is externally checked'})

    def model_crash(self,directory):
        injected=crash_model(self.client,self.expected_plan,self.live['process_host'],directory)
        # The next normal task uses native RuntimeBundle preparation, not driver
        # model startup. Successful recovery is recorded only after that task.
        return injected

    def corrupt(self,directory):
        self.close();archive=self.native/f'archive-{self.serial}'
        request=self.native/f'archive-request-{self.serial}.json';retain(request,{'workspace':str(self.workspace),'archive':str(archive)})
        self.run_command([str(self.archive_binary),str(request)],directory/'archive.log',120)
        saved=json.load(open(directory/'archive.log'));sha=saved['receipt']['cutover_recovery_sha256']
        database=self.state/'store/storage.sqlite'
        with database.open('r+b') as stream:stream.write(b'P14 CORRUPTED DB\0');stream.flush();os.fsync(stream.fileno())
        broken=digest(database)
        self.client_serial+=1
        refused=NativeClient(self.binary,self.workspace,self.state,self.config,self.out/f'refused-client-{self.client_serial}',self.heap_library)
        deadline=time.monotonic()+1200
        while refused.process.poll() is None:
            if time.monotonic()>deadline:raise RuntimeError('Corrupt startup did not fail within its bounded admission')
            time.sleep(.05)
        refused.reader.join(timeout=10)
        code=refused.process.returncode
        error=(refused.output/'stderr.log').read_text()
        try:refused.close()
        except RuntimeError as failure:
            if str(failure)!=f'CLI drain failed: {code}':raise
        retain(directory/'refused-startup.json',{'exit_code':code,'error':error,'log':ref(refused.output/'stderr.log')})
        if code==0 or 'NotADatabase' not in error or digest(database)!=broken:raise RuntimeError('Corrupt state was not specifically rejected without rewrite')
        retained=self.native/f'corrupt-state-{self.serial}';staged=self.native/f'restored-{self.serial}';request=self.native/f'restore-request-{self.serial}.json'
        retain(request,{'workspace':str(self.workspace),'archive':str(archive),'restore_sha256':sha,'retained_corrupt_state':str(retained),'restored_staging':str(staged)})
        self.run_command([str(self.archive_binary),str(request)],directory/'restore.log',120)
        if digest(retained/'store/storage.sqlite')!=broken:raise RuntimeError('Failed database bytes were not retained')
        self.open();artifacts_intact(self.state,self.seed)
        return self.remember_scenario('corrupt-state-recovery',directory,{'refused_startup_exit_code':code,'corrupt_database_sha256':broken,'native_archive_sha256':sha,'archive_receipt':ref(directory/'archive.log'),'restore_receipt':ref(directory/'restore.log')})

    def final_preservation(self):
        self.run_command([str(self.cutover),'audit-cutover','/home/alex/Projects/Cruzible'],self.out/'final-preservation.json',1800)

    def run(self):
        self.started=time.monotonic_ns();duration=self.args.hours*3600
        start={'version':1,'status':'started','start_monotonic_ns':self.started,'hours':self.args.hours,'input_sha256':input_digest(),'package_sha256':self.package_sha,'heap_growth_budget_bytes_per_hour':8*1024*1024,'activity_policy':POLICY,'warmup_seconds':1800,'token_accounting':'externalized-content-once','externalized_tokens':self.seed['token_count'],'fault_schedule':{'renderer_after_tasks':3,'branch_and_fork_after_tasks':2,'corruption_after_seconds':3600,'model_crash_after_seconds':duration/4,'host_crash_after_seconds':duration/2},'bound_inputs':self.bound,'claim_review':'Every final assistant/source claim requires retained agent review after or during the run; missing review cannot pass final quality.'}
        retain(self.out/'started.json',start);self.sampler.start();failure=None;end=None
        try:
            self.activity('admission',lambda d:self.open())
            self.activity('resume',lambda d:self.remember_scenario('resume',d,self.control('resume',{'session':self.seed['session']})))
            faults_done=set();index=0;pending_model_fault=None
            while (time.monotonic_ns()-self.started)/1e9<duration or index<self.args.hours*2:
                elapsed=(time.monotonic_ns()-self.started)/1e9
                if index==2 and 'branch' not in faults_done:
                    self.activity('branch',self.branch_fork);faults_done.add('branch')
                if elapsed>=3600 and 'corruption' not in faults_done:
                    self.activity('corrupt-state-recovery',self.corrupt);faults_done.add('corruption')
                if elapsed>=duration/4 and 'model' not in faults_done:
                    pending_model_fault=self.activity('model-crash',self.model_crash);faults_done.add('model')
                if elapsed>=duration/2 and 'host' not in faults_done and index!=4:
                    self.activity('host-crash',self.host_crash);faults_done.add('host')
                self.task(index,through_renderer=index==3)
                if pending_model_fault is not None:
                    # Actual successful task after the owned model exit proves
                    # ordinary runtime preparation/recovery was usable.
                    path=self.out/'model-recovery.json';retain(path,{'injection':pending_model_fault,'successful_next_task':self.tasks[-1],'status':'passed'})
                    self.scenarios.append({'scenario':'model-crash','executed':True,'passed':True,'evidence':[ref(path)]});pending_model_fault=None
                if index==2 or (index>3 and index%6==5):
                    self.activity('resume',lambda d:self.remember_scenario('resume',d,self.control('resume',{'session':self.seed['session']})))
                if index%6==5:self.activity('checkpoint-verifier',lambda d:{'all_original_artifacts_intact':self.evidence_bound()})
                index+=1
            self.evidence_bound();self.sampler.sample();end=time.monotonic_ns();self.sampler.close();self.close()
            memory=analyze_memory(self.sampler.samples,self.started,start['warmup_seconds'],start['heap_growth_budget_bytes_per_hour'])
            retain(self.out/'memory-analysis.json',memory)
            if memory['status']!='passed':raise RuntimeError('Per-host post-warmup heap growth failed its predeclared budget')
            self.final_preservation()
            self.evidence_bound()
        except (Exception, KeyboardInterrupt) as error:
            failure=repr(error);traceback.print_exc()
        finally:
            if end is None:end=time.monotonic_ns()
            if self.sampler and not self.sampler.stopped.is_set():
                try:self.sampler.close()
                except Exception as error:failure=failure or repr(error)
            if self.client is not None:
                try:self.close()
                except Exception as error:failure=failure or repr(error)
            self.scenarios.append({'scenario':'source-edit','executed':bool(self.tasks),'passed':bool(self.tasks),'evidence':[t['verification'] for t in self.tasks]})
            report={'version':1,'status':'failed' if failure else 'awaiting-claim-review','kind':f'duration-{self.args.hours}h','input_sha256':start['input_sha256'],'package_sha256':self.package_sha,'start_monotonic_ns':self.started,'end_monotonic_ns':end,'start_receipt':ref(self.out/'started.json'),'externalized_tokens':self.seed['token_count'],'token_accounting':'externalized-content-once','activity_policy':POLICY,'heap_growth_budget_bytes_per_hour':start['heap_growth_budget_bytes_per_hour'],'warmup_seconds':start['warmup_seconds'],'activities':self.activities,'sessions':self.scenarios,'samples':self.sampler.samples,'memory_samples':ref(self.out/'memory.jsonl'),'memory_analysis':ref(self.out/'memory-analysis.json') if (self.out/'memory-analysis.json').exists() else None,'tasks':self.tasks,'commands':[c for c in self.commands if c['exit_code']==0],'declared_failed_commands':[c for c in self.commands if c['exit_code']!=0],'error':failure,'quality_review_pending':True,'release_acceptance':False}
            retain(self.out/'duration-result.json',report)
        if failure:raise RuntimeError(failure)
        print(f'{self.args.hours}h real workload completed; retained claim reviews and final gate validation remain required.')

if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('output',type=Path);parser.add_argument('--preflight',type=Path,required=True);parser.add_argument('--hours',type=int,choices=[8,24],required=True);parser.add_argument('--heap-library',type=Path,required=True)
    Run(parser.parse_args()).run()
