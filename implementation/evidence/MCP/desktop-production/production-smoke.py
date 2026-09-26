import sys,os,json,subprocess,time,struct,hashlib,ctypes,ctypes.util,threading,http.server
from pathlib import Path
sys.path.insert(0,'tools')
from release_smoke import tree,stop
root=Path('/home/alex/Projects/AG1');base=root/'.local/mcp-b6';work=base/'production-smoke';work.mkdir(mode=0o700)
package=json.loads((base/'package.json').read_text());release=(base/'validated-package/current').resolve();assert release.is_dir()
requests=[]
class Sink(http.server.BaseHTTPRequestHandler):
 def log_message(self,*args): pass
 def do_POST(self):
  requests.append({'method':self.command,'path':self.path});self.send_response(503);self.end_headers()
sink=http.server.ThreadingHTTPServer(('127.0.0.1',0),Sink);thread=threading.Thread(target=sink.serve_forever,daemon=True);thread.start()
config={'models':[{'provider':'fixture','id':'startup-fixture','api':'openai-completions','baseUrl':f'http://127.0.0.1:{sink.server_port}','contextWindow':32768,'maxTokens':512,'input':['text'],'reasoning':False}],'selected':'startup-fixture','credential_env':None,'system_prompt':'Owned production MCP manager smoke; no generation permitted.','fixture':True,'policy_mode':'interactive'}
configuration=work/'host.json';configuration.write_text(json.dumps(config));configuration.chmod(0o600)
registry=work/'authority';registry.mkdir(mode=0o700)
display=next(n for n in range(500,600) if not Path(f'/tmp/.X{n}-lock').exists() and not Path(f'/tmp/.X11-unix/X{n}').exists())
auth=work/'display.auth';auth.write_bytes(struct.pack('>H',65535)+b''.join(struct.pack('>H',len(v))+v for v in [b'',str(display).encode(),b'MIT-MAGIC-COOKIE-1',os.urandom(16)]));auth.chmod(0o600)
env={k:os.environ[k] for k in ['HOME','PATH','LANG'] if k in os.environ}
env.update(DISPLAY=f':{display}',XAUTHORITY=str(auth),GDK_BACKEND='x11',LIBGL_ALWAYS_SOFTWARE='1',WEBKIT_DISABLE_DMABUF_RENDERER='1',AG1_NATIVE_CONFIG=str(configuration),AG1_FIXTURE_PROVIDER='1',AG1_DESKTOP_STATE=str(work/'state'),XDG_CONFIG_HOME=str(work/'config'),XDG_CACHE_HOME=str(work/'cache'),XDG_DATA_HOME=str(work/'data'))
app=xvfb=None;owned={};report={'status':'failed','production_binary_sha256':hashlib.sha256((release/'bin/ag1-desktop').read_bytes()).hexdigest(),'package_sha256':package['archive']['sha256'],'scope':'Normal production binary, private fixture configuration/display/state/authority; manager navigation without inference','screenshots':[]}
def screenshot(name):
 subprocess.run(['import','-display',f':{display}','-window','root',str(work/name)],env=env,check=True,timeout=20);report['screenshots'].append(str(work/name))
def click(x,y):
 os.environ['XAUTHORITY']=str(auth)
 x11=ctypes.CDLL(ctypes.util.find_library('X11'));xt=ctypes.CDLL(ctypes.util.find_library('Xtst'))
 x11.XOpenDisplay.argtypes=[ctypes.c_char_p];x11.XOpenDisplay.restype=ctypes.c_void_p
 x11.XCloseDisplay.argtypes=[ctypes.c_void_p];x11.XFlush.argtypes=[ctypes.c_void_p]
 xt.XTestFakeMotionEvent.argtypes=[ctypes.c_void_p,ctypes.c_int,ctypes.c_int,ctypes.c_int,ctypes.c_ulong]
 xt.XTestFakeButtonEvent.argtypes=[ctypes.c_void_p,ctypes.c_uint,ctypes.c_int,ctypes.c_ulong]
 d=x11.XOpenDisplay(env['DISPLAY'].encode());assert d
 try:
  assert xt.XTestFakeMotionEvent(d,-1,x,y,0)
  assert xt.XTestFakeButtonEvent(d,1,1,0);assert xt.XTestFakeButtonEvent(d,1,0,0);x11.XFlush(d)
 finally:x11.XCloseDisplay(d)
try:
 xvfb=subprocess.Popen([str(root/'.local/p11/xvfb/usr/bin/Xvfb'),f':{display}','-screen','0','1440x900x24','-nolisten','tcp','-auth',str(auth)],stdout=(work/'xvfb.log').open('x'),stderr=subprocess.STDOUT)
 deadline=time.monotonic()+10
 while not Path(f'/tmp/.X11-unix/X{display}').exists():
  assert xvfb.poll() is None and time.monotonic()<deadline;time.sleep(.1)
 command=['/usr/bin/bwrap','--die-with-parent','--unshare-pid','--ro-bind','/','/','--proc','/proc','--dev-bind','/dev','/dev','--bind',str(work),str(work),'--bind',str(registry),str(Path.home()/'.local/state/ag1/authority-v1'),'--tmpfs','/tmp','--ro-bind','/tmp/.X11-unix','/tmp/.X11-unix','--',str(release/'run-ag1')]
 app=subprocess.Popen(command,cwd=work,env=env,stdout=(work/'desktop.log').open('x'),stderr=subprocess.STDOUT)
 time.sleep(5);assert app.poll() is None,'Production app exited';screenshot('production-home.png')
 # Coordinates come from the inspected 1440x900 native MCP screenshots. All
 # XTest events target only the authenticated private X server, never the user.
 click(42,830);time.sleep(3);assert app.poll() is None;screenshot('production-mcp.png')
 assert (work/'state/mcp-store').is_dir(),'Normal production manager did not open'
 click(536,266);time.sleep(1);screenshot('production-mcp-add.png')
 assert requests==[],'Manager navigation started inference'
 rows=tree(app.pid);owned.update({r['pid']:r['start'] for r in rows});report.update(status='passed_pending_screenshot_review',processes=rows,provider_requests=requests,visual_test_broker=False,input='Synthetic XTest pointer events on private display; not physical input')
finally:
 if app is not None: report['leaked_owned_children']=stop(app,owned)
 if xvfb is not None and xvfb.poll() is None:xvfb.terminate();xvfb.wait(timeout=10)
 sink.shutdown();sink.server_close();thread.join(timeout=3)
 (work/'result.json').write_text(json.dumps(report,indent=2)+'\n')
print(json.dumps(report));assert not report.get('leaked_owned_children')
