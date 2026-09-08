"""Sequential empty-home Linux shell comparison; no model or startup certification."""
import argparse
import ctypes as c
import importlib.util
import os
from pathlib import Path
import struct
import subprocess
import sys
import time
os.umask(0o077)
sys.path.insert(0,'tools')
from common import ROOT,GateError,read_json,write_json
from release_gate import ref
from release_smoke import tree,stop

def fullscreen(display):
    x=c.CDLL('libX11.so.6')
    x.XOpenDisplay.argtypes=[c.c_char_p];x.XOpenDisplay.restype=c.c_void_p
    x.XDefaultRootWindow.argtypes=[c.c_void_p];x.XDefaultRootWindow.restype=c.c_ulong
    x.XQueryTree.argtypes=[c.c_void_p,c.c_ulong,c.POINTER(c.c_ulong),c.POINTER(c.c_ulong),c.POINTER(c.POINTER(c.c_ulong)),c.POINTER(c.c_uint)]
    x.XGetGeometry.argtypes=[c.c_void_p,c.c_ulong,c.POINTER(c.c_ulong),c.POINTER(c.c_int),c.POINTER(c.c_int),c.POINTER(c.c_uint),c.POINTER(c.c_uint),c.POINTER(c.c_uint),c.POINTER(c.c_uint)]
    x.XMoveResizeWindow.argtypes=[c.c_void_p,c.c_ulong,c.c_int,c.c_int,c.c_uint,c.c_uint]
    x.XSync.argtypes=[c.c_void_p,c.c_int];x.XCloseDisplay.argtypes=[c.c_void_p];x.XFree.argtypes=[c.c_void_p]
    d=x.XOpenDisplay(display.encode());assert d
    try:
        parent=c.c_ulong();base=c.c_ulong();children=c.POINTER(c.c_ulong)();count=c.c_uint()
        assert x.XQueryTree(d,x.XDefaultRootWindow(d),c.byref(base),c.byref(parent),c.byref(children),c.byref(count))
        selected=[]
        for i in range(count.value):
            a,b=c.c_int(),c.c_int();w,h,border,depth=[c.c_uint() for _ in range(4)]
            if x.XGetGeometry(d,children[i],c.byref(base),c.byref(a),c.byref(b),c.byref(w),c.byref(h),c.byref(border),c.byref(depth)) and w.value>=400 and h.value>=300:
                selected.append(children[i])
        x.XFree(children)
        assert len(selected)==1,selected
        x.XMoveResizeWindow(d,selected[0],0,0,1440,900);x.XSync(d,0)
        return {'width':1440,'height':900,'private_x11_window':selected[0]}
    finally:x.XCloseDisplay(d)

def main(output):
    base=ROOT/'.local/p14-shell-pair'
    if base.is_symlink():raise GateError('Private shell output base cannot be redirected')
    base.mkdir(exist_ok=True,mode=0o700)
    root=output.absolute()
    if root.exists() or root.parent.resolve()!=base.resolve():
        raise GateError('Use a new direct child of .local/p14-shell-pair')
    for path in Path('/proc').iterdir():
        if not path.name.isdigit():continue
        try:
            if (path/'comm').read_text().strip()=='llama-server':
                raise GateError('Finish the owned model workload before shell measurement')
        except FileNotFoundError:pass
    root.mkdir(mode=0o700)
    package=read_json(ROOT/'implementation/evidence/P14/package.json')
    spec=importlib.util.spec_from_file_location('installer',ROOT/'product/release/install.py')
    installer=importlib.util.module_from_spec(spec);spec.loader.exec_module(installer)
    
    installed=installer.install(ROOT/package['archive']['path'],package['archive']['sha256'],root/'installation')
    observations=[]
    for runner in ['dyad','ag1']:
        out=root/runner;out.mkdir(mode=0o700);(out/'runtime').mkdir(mode=0o700)
        display=next(n for n in range(100,500) if not Path(f'/tmp/.X{n}-lock').exists() and not Path(f'/tmp/.X11-unix/X{n}').exists())
        auth=out/'display.auth'
        auth.write_bytes(struct.pack('>H',65535)+b''.join(struct.pack('>H',len(v))+v for v in [b'',str(display).encode(),b'MIT-MAGIC-COOKIE-1',os.urandom(16)]))
        env=os.environ.copy()
        for key in ['WAYLAND_DISPLAY','DBUS_SESSION_BUS_ADDRESS','DISPLAY','XAUTHORITY']:env.pop(key,None)
        env.update(DISPLAY=f':{display}',XAUTHORITY=str(auth),LIBGL_ALWAYS_SOFTWARE='1',GDK_BACKEND='x11',
          XDG_SESSION_TYPE='x11',XDG_RUNTIME_DIR=str(out/'runtime'),XDG_CONFIG_HOME=str(out/'config'),
          XDG_CACHE_HOME=str(out/'cache'),XDG_DATA_HOME=str(out/'data'),
          AG1_DESKTOP_STATE=str(out/'desktop-state'),WEBKIT_DISABLE_DMABUF_RENDERER='1')
        # Xlib uses only this task's authenticated display for geometry operations.
        os.environ['XAUTHORITY']=str(auth)
        command=['bwrap','--ro-bind','/','/']
        if runner=='dyad':command+=['--tmpfs','/home/alex','--ro-bind',str(ROOT),str(ROOT)]
        command+=['--bind',str(root),str(root),'--tmpfs','/tmp','--ro-bind','/tmp/.X11-unix','/tmp/.X11-unix',
          '--dev','/dev','--proc','/proc','--unshare-net','--unshare-pid','--new-session','--die-with-parent']
        if runner=='dyad':command += [str(ROOT/'reference/dyad-executable/out/dyad-linux-x64/dyad'),'--ozone-platform=x11','--disable-gpu','--user-data-dir='+str(out/'user-data'),'--no-first-run']
        else:command += [str(installed/'run-ag1')]
        report={'runner':runner,'status':'running','argv':command,'samples':[],'network':'private namespace',
          'GPU_devices':'unavailable in private /dev','warmup_seconds':60,'measurement_seconds':300,'startup_measurement':False}
        write_json(out/'started.json',report)
        xvfb=desktop=None;owned={}
        try:
            xvfb=subprocess.Popen([str(ROOT/'.local/p11/xvfb/usr/bin/Xvfb'),f':{display}','-screen','0','1440x900x24','-nolisten','tcp','-auth',str(auth)],stdout=(out/'xvfb.log').open('x'),stderr=subprocess.STDOUT)
            deadline=time.monotonic()+10
            while not Path(f'/tmp/.X11-unix/X{display}').exists():
                assert xvfb.poll() is None and time.monotonic()<deadline
                time.sleep(.05)
            desktop=subprocess.Popen(command,cwd=out,env=env,stdout=(out/'desktop.log').open('x'),stderr=subprocess.STDOUT)
            time.sleep(15);assert desktop.poll() is None
            owned.update({r['pid']:r['start'] for r in tree(desktop.pid)})
            report['geometry']=fullscreen(f':{display}')
            time.sleep(45)
            subprocess.run(['import','-display',f':{display}','-window','root',str(out/'shell.png')],env=env,check=True,timeout=30)
            report['screenshot']=ref(out/'shell.png')
            start=time.monotonic()
            while True:
                assert desktop.poll() is None
                rows=tree(desktop.pid);owned.update({r['pid']:r['start'] for r in rows})
                report['samples'].append({'elapsed_seconds':time.monotonic()-start,'pss_bytes':sum(r['pss_bytes'] for r in rows),
                  'cpu_ticks':sum(r['cpu_ticks'] for r in rows),'processes':rows,'system_load':os.getloadavg()})
                if report['samples'][-1]['elapsed_seconds']>=300:break
                time.sleep(5)
            first,last=report['samples'][0],report['samples'][-1]
            identities=lambda sample:{(r['pid'],r['start']) for r in sample['processes']}
            stable=all(identities(s)==identities(first) for s in report['samples'])
            assert stable,'Process churn requires different CPU accounting'
            report.update(status='measured',stable_process_tree=stable,
              idle_cpu_core_percent=(last['cpu_ticks']-first['cpu_ticks'])/os.sysconf('SC_CLK_TCK')/(last['elapsed_seconds']-first['elapsed_seconds'])*100,
              mean_pss_bytes=sum(s['pss_bytes'] for s in report['samples'])/len(report['samples']))
        finally:
            if desktop is not None:report['leaked_children']=stop(desktop,owned)
            if xvfb is not None and xvfb.poll() is None:xvfb.terminate();xvfb.wait(timeout=10)
            write_json(out/'observations.json',report)
        assert report['status']=='measured' and not report['leaked_children']
        observations.append(report)
        print(runner,report['mean_pss_bytes']/1024**2,report['idle_cpu_core_percent'],flush=True)
    old,new=observations
    write_json(ROOT/'implementation/evidence/P14/shell-pair.json',{'status':'measured','release_acceptance':False,
     'scope':'One sequential matched-size empty-home shell pair on private X11 with software rendering, network isolation, no GPU devices and no model. Shared workstation load remains; full repeated performance acceptance is separate.',
     'package':package['archive'],'driver':ref(Path(__file__).resolve()),
     'observations':[ref(root/r/'observations.json') for r in ['dyad','ag1']],
     'dyad_mean_pss_bytes':old['mean_pss_bytes'],'ag1_mean_pss_bytes':new['mean_pss_bytes'],
     'memory_reduction_percent':100*(1-new['mean_pss_bytes']/old['mean_pss_bytes']),
     'dyad_idle_cpu_core_percent':old['idle_cpu_core_percent'],'ag1_idle_cpu_core_percent':new['idle_cpu_core_percent']})


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('new_private_output',type=Path)
    main(parser.parse_args().new_private_output)
