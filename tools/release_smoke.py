#!/usr/bin/env python3
"""Measure an installed production Linux shell on a private software-rendered display.

No workspace or provider is opened. These observations cannot certify matched
Dyad performance, interactive startup, or session recovery.
"""
import importlib.util
import json
import os
from pathlib import Path
import signal
import struct
import subprocess
import time
import uuid

from common import ROOT, GateError, read_json, write_json
from release_gate import EVIDENCE, ref
from tool_gate import environment


def process(pid):
    try:
        base=Path('/proc')/str(pid)
        fields=(base/'stat').read_text().rsplit(')',1)[1].split()
        return {'pid':pid,'name':(base/'comm').read_text().strip(),'state':fields[0],
                'ppid':int(fields[1]),'group':int(fields[2]),'start':fields[19],
                'cpu_ticks':int(fields[11])+int(fields[12])}
    except (FileNotFoundError,ProcessLookupError):
        return None


def tree(leader):
    all_rows={int(p.name):process(int(p.name)) for p in Path('/proc').iterdir() if p.name.isdigit()}
    selected={leader}
    changed=True
    while changed:
        old=len(selected)
        selected.update(pid for pid,row in all_rows.items() if row and row['ppid'] in selected)
        changed=len(selected)!=old
    rows=[]
    for pid in selected:
        row=all_rows.get(pid)
        if not row:continue
        try:
            values=Path('/proc',str(pid),'smaps_rollup').read_text().splitlines()
            row['pss_bytes']=next(int(line.split()[1])*1024 for line in values if line.startswith('Pss:'))
            rows.append(row)
        except (FileNotFoundError, ProcessLookupError):pass
    return rows


def stop(child, identities):
    # Signal only authenticated children from this launch, with live pidfd identity checks.
    if child.poll() is None:
        child.terminate()
        try:child.wait(timeout=10)
        except subprocess.TimeoutExpired:
            child.kill();child.wait(timeout=10)
    for pid,start in identities.items():
        row=process(pid)
        if row and row['start']==start and row['state']!='Z':
            try:
                fd=os.pidfd_open(pid)
                current=process(pid)
                if current and current['start']==start:
                    signal.pidfd_send_signal(fd,signal.SIGTERM)
                os.close(fd)
            except ProcessLookupError:pass
    end=time.monotonic()+10
    while time.monotonic()<end:
        live=[pid for pid,start in identities.items() if (r:=process(pid)) and r['start']==start and r['state']!='Z']
        if not live:return []
        time.sleep(.1)
    return live


def main():
    os.umask(0o077)
    package=read_json(EVIDENCE/'package.json')
    output=ROOT/'.local/p14-shell-smoke'/uuid.uuid4().hex
    output.mkdir(parents=True,mode=0o700)
    spec=importlib.util.spec_from_file_location('installer',ROOT/'product/release/install.py')
    installer=importlib.util.module_from_spec(spec);spec.loader.exec_module(installer)
    installed=installer.install(ROOT/package['archive']['path'],package['archive']['sha256'],output/'installation')
    display=next(n for n in range(100,500) if not Path(f'/tmp/.X{n}-lock').exists() and not Path(f'/tmp/.X11-unix/X{n}').exists())
    auth=output/'display.auth'
    auth.write_bytes(struct.pack('>H',65535)+b''.join(struct.pack('>H',len(v))+v for v in [b'',str(display).encode(),b'MIT-MAGIC-COOKIE-1',os.urandom(16)]))
    _,env=environment()
    env.update(DISPLAY=f':{display}',XAUTHORITY=str(auth),GDK_BACKEND='x11',LIBGL_ALWAYS_SOFTWARE='1',
               WEBKIT_DISABLE_DMABUF_RENDERER='1',AG1_DESKTOP_STATE=str(output/'desktop-state'))
    xvfb=desktop=None;owned={};leaks=[]
    report={'status':'running','package_sha256':package['archive']['sha256'],
            'scope':'Installed release production binary, empty home, software-rendered private X11; no provider/workspace; unmatched exploratory metrics',
            'release_acceptance':False,'model_actions':0,'samples':[],'launches':[]}
    try:
        xvfb=subprocess.Popen([str(ROOT/'.local/p11/xvfb/usr/bin/Xvfb'),f':{display}','-screen','0','1440x900x24','-nolisten','tcp','-auth',str(auth)],stdout=(output/'xvfb.log').open('x'),stderr=subprocess.STDOUT)
        deadline=time.monotonic()+10
        while not Path(f'/tmp/.X11-unix/X{display}').exists():
            if xvfb.poll() is not None or time.monotonic()>deadline:raise GateError('Private X server failed')
            time.sleep(.05)
        for attempt in range(2):
            start=time.monotonic()
            desktop=subprocess.Popen([str(installed/'run-ag1')],cwd=output,env=env,stdout=(output/f'desktop-{attempt}.log').open('x'),stderr=subprocess.STDOUT)
            # Rendering is inspected via a screenshot; this sleep is explicitly NOT a startup metric.
            time.sleep(10)
            if desktop.poll() is not None:raise GateError('Installed desktop exited during shell smoke')
            rows=tree(desktop.pid);owned.update({r['pid']:r['start'] for r in rows})
            subprocess.run(['import','-display',f':{display}','-window','root',str(output/f'shell-{attempt}.png')],env=env,check=True,timeout=30)
            report['launches'].append({'attempt':attempt,'alive_at_seconds':time.monotonic()-start,'processes':rows,'screenshot':ref(output/f'shell-{attempt}.png')})
            if attempt==0:
                # Simulated abrupt shell exit has no session effects because none was opened.
                desktop.kill();desktop.wait(timeout=10)
                leaks+=stop(desktop,owned);desktop=None;owned={}
                continue
            time.sleep(50)
            began=time.monotonic()
            while True:
                rows=tree(desktop.pid);owned.update({r['pid']:r['start'] for r in rows})
                report['samples'].append({'elapsed_seconds':time.monotonic()-began,'pss_bytes':sum(r['pss_bytes'] for r in rows),'cpu_ticks':sum(r['cpu_ticks'] for r in rows),'processes':rows})
                if report['samples'][-1]['elapsed_seconds']>=300:break
                time.sleep(5)
            leaks+=stop(desktop,owned);desktop=None
        first,last=report['samples'][0],report['samples'][-1]
        stable={(r['pid'],r['start']) for r in first['processes']}=={(r['pid'],r['start']) for r in last['processes']}
        report.update(status='passed' if not leaks else 'failed',leaked_owned_children=leaks,
                      idle_seconds=last['elapsed_seconds']-first['elapsed_seconds'],
                      stable_process_tree=stable,
                      idle_cpu_core_percent=(last['cpu_ticks']-first['cpu_ticks'])/os.sysconf('SC_CLK_TCK')/(last['elapsed_seconds']-first['elapsed_seconds'])*100 if stable else None)
    finally:
        if desktop is not None:stop(desktop,owned)
        if xvfb is not None and xvfb.poll() is None:
            xvfb.terminate();xvfb.wait(timeout=10)
        write_json(output/'observations.json',report)
        write_json(EVIDENCE/'shell-smoke.json',{'status':report['status'],'release_acceptance':False,
                   'package_sha256':package['archive']['sha256'],'observation':ref(output/'observations.json'),
                   'screenshots':[r['screenshot'] for r in report['launches']],
                   'idle_cpu_core_percent':report.get('idle_cpu_core_percent'),'scope':report['scope']})
    if report['status']!='passed':raise GateError('Installed shell smoke failed')
    print(json.dumps({'status':report['status'],'idle_cpu_core_percent':report['idle_cpu_core_percent'],'leaked_owned_children':leaks}))


if __name__=='__main__':main()
