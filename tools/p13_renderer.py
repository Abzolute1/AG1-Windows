"""Owned WebKit cutover/resnapshot measurement; fixture and live modes stay distinct."""
from __future__ import annotations
import argparse
import http.server
import json
import os
from pathlib import Path
import re
import secrets
import struct
import subprocess
import sys
import threading
import time

from common import ROOT, GateError, read_json, sha256, verify_original
from phase_gate import verify_dependencies
from project_gate import verify_tooling
from runtime_gate import preservation
from p13_acceptance import fingerprint
from cutover_gate import inputs
from client_visual import terminate
sys.path.insert(0,str(ROOT/'tools/probes'))
import desktop_visual as ui


def retain(path, value):
    with path.open('x') as out:
        json.dump(value,out,separators=(',',':'));out.flush();os.fsync(out.fileno())
    path.chmod(0o600)
    fd=os.open(path.parent,os.O_RDONLY)
    try:os.fsync(fd)
    finally:os.close(fd)
    return {'path':str(path),'sha256':sha256(path)}


class Provider(http.server.BaseHTTPRequestHandler):
    protocol_version='HTTP/1.1'
    def log_message(self,*args):pass
    def do_POST(self):
        size=int(self.headers.get('Content-Length','0'))
        if not 0<size<=4*1024*1024:raise RuntimeError('Fixture request size')
        body=json.loads(self.rfile.read(size))
        markers=re.findall(r'P13_RENDERER_[A-F0-9]{8}',json.dumps(body))
        if not markers:raise RuntimeError('Fixture marker missing')
        marker=markers[-1]
        self.send_response(200);self.send_header('Content-Type','text/event-stream');self.send_header('Connection','close');self.end_headers()
        try:
            for _ in range(60):
                chunk={'choices':[{'delta':{'content':marker+'\n'},'finish_reason':None}]}
                self.wfile.write(('data: '+json.dumps(chunk)+'\n\n').encode());self.wfile.flush();time.sleep(.05)
            self.wfile.write(('data: '+json.dumps({'choices':[{'delta':{},'finish_reason':'stop'}],
                'usage':{'prompt_tokens':1000,'completion_tokens':600}})+'\n\ndata: [DONE]\n\n').encode());self.wfile.flush()
        except (BrokenPipeError,ConnectionResetError):pass
        self.close_connection=True


def validate(value, fixture):
    samples=value.get('observations',[])
    freeze=value.get('freeze',{})
    before=value.get('before',{}).get('snapshot',{})
    after=value.get('after',{}).get('snapshot',{})
    if (value.get('version')!=1 or value.get('status')!='passed' or value.get('fixture') is not fixture
            or value.get('case')!='renderer-resnapshot' or value.get('executed') is not True
            or type(value.get('elapsed_ms')) is not int or value['elapsed_ms']<0
            or not value.get('screenshot',{}).get('path') or not value.get('screenshot',{}).get('sha256')
            or value.get('renderer')!='owned-tauri-webkit' or value.get('dom_marker_visible') is not True
            or value.get('tools_restored') is not True or value.get('desktop_exit_code')!=0 or value.get('authority_quiescent') is not True
            or type(freeze.get('elapsed_ms')) not in (int,float) or freeze['elapsed_ms']<30000
            or before.get('scope')!=after.get('scope') or not before.get('scope')
            or before.get('scope',{}).get('session')!=value.get('selection',{}).get('session')
            or before.get('workspace_path')!=value.get('selection',{}).get('workspace')
            or after.get('fixture_provider') is not fixture or before.get('fixture_provider') is not fixture
            or after.get('status')!='idle' or int(after.get('revision',0))<=int(before.get('revision',0))):
        raise GateError('Incomplete or unrelated real-renderer resnapshot evidence')
    if (not samples or any(not isinstance(s,dict) or type(s.get('since_arm_ms')) is not int or s['since_arm_ms']<0 for s in samples)
            or any(a['since_arm_ms']>=b['since_arm_ms'] for a,b in zip(samples,samples[1:]))):
        raise GateError('Missing or unordered independent native observations')
    running=[s for s in samples if s.get('status')=='streaming' and s.get('since_arm_ms',0)>=500]
    idle=[s for s in samples if s.get('status')=='idle' and s.get('marker_seen') is True]
    if not any(a['since_arm_ms']<b['since_arm_ms']<freeze['elapsed_ms']-500 for a in running for b in idle):
        raise GateError('No observed native completion while the renderer was frozen')
    if any(s.get('scope')!=before['scope'] or s.get('error') is not None for s in samples):
        raise GateError('Native observation changed scope or required recovery')


def run(output, workspace=None, rehearse=False, timeout=900):
    verify_dependencies('P13');verify_tooling();pins=verify_original();preserved=preservation()
    source=inputs();os.umask(0o077)
    output=output.absolute()
    if (not output.parent.resolve().is_relative_to((ROOT/'.local').resolve()) or output.exists()
            or output.parent.is_symlink()):raise GateError('Use a new private evidence directory under .local')
    build='debug' if rehearse else 'release'
    cli=ROOT/f'product/target/{build}/ag1-cutover'
    desktop_bin=ROOT/f'product/target/{build}/ag1-desktop'
    server=desktop=xvfb=None
    if not rehearse:
        if workspace is None:raise GateError('A selected workspace is required for a real renderer run')
        # Fail before reserving output or opening a client if runtime custody is unavailable.
        audit=json.loads(subprocess.run([str(cli),'audit-cutover',str(workspace)],capture_output=True,text=True,check=True,timeout=1800).stdout)
        if audit.get('fixture') is not False:raise GateError('Production renderer run requires production authority')
        transfer=audit['authority']['transfer']
        selection={'workspace':audit['authority']['workspace'],'session':transfer['session'],
                   'original_state':transfer['original_state'],'destination':transfer['destination']}
        for path in [*selection.values(), transfer.get('archive'),
                     audit.get('source_snapshot',{}).get('path'),
                     (audit.get('runtime_acceptance') or {}).get('precommit_recovery'),
                     (audit.get('rollback_archive') or {}).get('archive')]:
            if path and Path(path).is_absolute() and (output.resolve().is_relative_to(Path(path).resolve()) or Path(path).resolve().is_relative_to(output.resolve())):
                raise GateError('Renderer evidence must be outside retained source/state/archive trees')
    output.mkdir(mode=0o700)
    if rehearse:
        if workspace is not None:raise GateError('Rehearsal creates its own new workspace')
        server=http.server.ThreadingHTTPServer(('127.0.0.1',0),Provider)
        threading.Thread(target=server.serve_forever,daemon=True).start()
        seeded=subprocess.run([str(ROOT/'product/target/debug/examples/cutover-renderer-seed'),
            str(output/'fixture'),f'http://127.0.0.1:{server.server_port}'],capture_output=True,text=True,timeout=60)
        retain(output/'seed.json',{'exit_code':seeded.returncode,'stdout':seeded.stdout,'stderr':seeded.stderr,'fixture':True})
        if seeded.returncode:raise GateError('Fixture migration failed; see private seed.json')
        selection=json.loads(seeded.stdout)
        selection.pop('fixture')
    workspace=Path(selection['workspace']);state=output/'desktop-state';state.mkdir(mode=0o700)
    display=next(n for n in range(100,500) if not Path(f'/tmp/.X{n}-lock').exists() and not Path(f'/tmp/.X11-unix/X{n}').exists())
    auth=output/'display.auth';cookie=os.urandom(16)
    auth.write_bytes(struct.pack('>H',65535)+b''.join(struct.pack('>H',len(v))+v for v in [b'',str(display).encode(),b'MIT-MAGIC-COOKIE-1',cookie]))
    auth.chmod(0o600)
    report={'version':1,'status':'running','fixture':rehearse,'selection':selection,'input_sha256':fingerprint(source),
            'case':'renderer-resnapshot','operation':'renderer-resnapshot','renderer':'owned-tauri-webkit','executed':True,
            'phase_acceptance':False,'automatic_retry_allowed':False,'clock':'monotonic','observations':[]}
    start=retain(output/'started.json',{'version':1,'status':'started','arguments':['renderer-resnapshot',str(workspace)],
        'automatic_retry_allowed':False,'executable_sha256':sha256(desktop_bin),'input_sha256':fingerprint(source)})
    began=time.monotonic();failure=None;resnapshot_start=resnapshot_finish=None
    def native():return ui.request({'native_view':True})
    def control(command):
        result=ui.async_js('const invoke=window.__TAURI_INTERNALS__.invoke;const current=await invoke("bridge_view",{});'
            'return await invoke("bridge_command",{envelope:{version:current.snapshot.version,id:crypto.randomUUID(),'
            'scope:current.snapshot.scope,expected_revision:current.snapshot.revision,command:'+json.dumps(command)+',extension:null}});')
        if result.get('status')!='accepted':raise GateError('Renderer control was not accepted')
    try:
        with (output/'xvfb.log').open('x') as log:
            xvfb=subprocess.Popen([str(ROOT/'.local/p11/xvfb/usr/bin/Xvfb'),f':{display}','-screen','0','1440x900x24',
                '-nolisten','tcp','-auth',str(auth)],stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
        deadline=time.monotonic()+10
        while not Path(f'/tmp/.X11-unix/X{display}').exists():
            if xvfb.poll() is not None or time.monotonic()>deadline:raise GateError('Private display startup failed')
            time.sleep(.05)
        env=dict(os.environ,DISPLAY=f':{display}',XAUTHORITY=str(auth),GDK_BACKEND='x11',LIBGL_ALWAYS_SOFTWARE='1',
            WEBKIT_DISABLE_DMABUF_RENDERER='1',AG1_DESKTOP_STATE=str(state),AG1_VISUAL_SOCKET=str(output/'control.sock'))
        for key in ['AG1_NATIVE_CONFIG','AG1_FIXTURE_PROVIDER','AG1_CUTOVER_WORKSPACE']:env.pop(key,None)
        env['AG1_FIXTURE_PROVIDER' if rehearse else 'AG1_CUTOVER_WORKSPACE']='1' if rehearse else str(workspace)
        with (output/'desktop.log').open('x') as log:
            desktop=subprocess.Popen([str(desktop_bin)],cwd=ROOT,env=env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
        deadline=time.monotonic()+600
        while not (output/'control.sock').exists():
            if desktop.poll() is not None or time.monotonic()>deadline:raise GateError('Owned desktop startup failed')
            time.sleep(.1)
        ui.SOCKET=output/'control.sock';ui.wait('document.body.innerText.includes("What will you work on?")',30)
        ui.js('document.querySelector("button.open-card").click();true')
        ui.wait('!!document.querySelector(".dialog input")');ui.text('input[aria-label="Project folder"]',str(workspace))
        ui.js('document.querySelector(".dialog form").requestSubmit();true')
        ui.wait('!!document.querySelector('+json.dumps('textarea[aria-label="Message AG1"]')+')',30 if rehearse else timeout)
        initial=native();report['before']=initial['view'];prior_tools=initial['tools']
        control({'kind':'runtime','action':'tools','args':{'names':[]}})
        marker='P13_RENDERER_'+secrets.token_hex(4).upper()
        ui.text('textarea[aria-label="Message AG1"]',f'Reply with {marker} on each of 32 separate lines. Use no tools and add no explanation.')
        ui.click('Send message')
        ui.wait('document.querySelector(".streaming pre")?.textContent.includes('+json.dumps(marker)+') || !!document.querySelector(".error")',timeout)
        if ui.js('!!document.querySelector(".error")'):raise GateError('Native renderer workflow failed before streaming')
        freeze_ms=30000 if rehearse else 60000
        ui.js('window.__p13Freeze=null;setTimeout(()=>{const began=performance.now();while(performance.now()-began<'+str(freeze_ms)+'){};window.__p13Freeze={elapsed_ms:performance.now()-began};},0);true')
        armed=time.monotonic();print('P13 renderer: observing native host while its owned WebKit renderer is frozen',flush=True)
        while time.monotonic()-armed<(freeze_ms+500)/1000:
            view=native()['view'];snapshot=view['snapshot']
            report['observations'].append({'since_arm_ms':int((time.monotonic()-armed)*1000),'scope':snapshot['scope'],
                'status':snapshot['status'],'revision':snapshot['revision'],'watermark':snapshot['watermark'],'error':view['error'],
                'marker_seen':marker in json.dumps(view)})
            time.sleep(.1)
        report['freeze']=ui.js('window.__p13Freeze')
        resnapshot_start=retain(output/'resnapshot-started.json',{'version':1,'status':'started',
            'arguments':['renderer-resnapshot',str(workspace)],'automatic_retry_allowed':False,'executable_sha256':sha256(desktop_bin)})
        resnapshot_began=time.monotonic()
        ui.wait('!document.querySelector(".streaming")',30)
        report['after']=native()['view']
        report['dom_marker_visible']=bool(ui.js('document.body.innerText.includes('+json.dumps(marker)+')'))
        report['elapsed_ms']=int((time.monotonic()-resnapshot_began)*1000)
        resnapshot_finish=retain(output/'resnapshot-finished.json',{'version':1,'status':'command_succeeded',
            'started_sha256':resnapshot_start['sha256'],'elapsed_ms':report['elapsed_ms'],'automatic_retry_allowed':False})
        ui.screenshot('resnapshot')
        report['screenshot']={'path':str(output/'resnapshot.png'),'sha256':sha256(output/'resnapshot.png')}
        control({'kind':'runtime','action':'tools','args':{'names':prior_tools}})
        report['tools_restored']=native()['tools']==prior_tools
        # The quit IPC drains this owned host. A lost GUI acknowledgement is
        # resolved by observed exit and the quiescent durable authority below.
        ui.js('setTimeout(()=>window.__TAURI_INTERNALS__.invoke("quit",{}),0);true')
        desktop.wait(timeout=30);report['desktop_exit_code']=desktop.returncode
        record=json.loads(subprocess.run([str(cli),'inspect',str(workspace)],capture_output=True,text=True,check=True,timeout=15).stdout)['authority']
        report['authority_quiescent']=bool(record and record['owner']=='native' and record['quiescent'] and record['state']==selection['destination'])
        report['status']='passed';validate(report,rehearse)
    except Exception as error:
        report['status']='failed';failure=type(error).__name__+': '+str(error)
    finally:
        if desktop and desktop.poll() is None:
            try:ui.js('setTimeout(()=>window.__TAURI_INTERNALS__.invoke("quit",{}),0);true');desktop.wait(timeout=30)
            except Exception:pass
        terminate(desktop);terminate(xvfb)
        if server:server.shutdown();server.server_close()
        report['probe_elapsed_ms']=int((time.monotonic()-began)*1000)
        if pins!=verify_original() or preserved!=preservation() or source!=inputs():
            report['status']='failed';failure='Preservation or source inputs changed during measurement'
        finish=retain(output/'result.json',{'version':1,'status':'command_succeeded' if report['status']=='passed' else 'command_failed',
            'started_sha256':start['sha256'],'elapsed_ms':report['probe_elapsed_ms'],'error':failure,'automatic_retry_allowed':False})
        report['started_receipt']=resnapshot_start or start;report['finished_receipt']=resnapshot_finish or finish
        retain(output/'observation.json',report)
    print(json.dumps({'status':report['status'],'fixture':rehearse,'evidence':str(output),'phase_acceptance':False}),flush=True)
    if failure:raise GateError(failure)
    return report


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('new_private_output',type=Path);parser.add_argument('--workspace',type=Path)
    parser.add_argument('--rehearse',action='store_true');parser.add_argument('--timeout',type=int,default=900)
    args=parser.parse_args()
    try:
        if not 1<=args.timeout<=1800:raise GateError('Invalid renderer timeout')
        run(args.new_private_output,args.workspace,args.rehearse,args.timeout)
    except (GateError,OSError,ValueError,subprocess.SubprocessError) as error:
        print('P13 renderer did not pass; retained evidence: '+str(error),file=sys.stderr);raise SystemExit(1)
