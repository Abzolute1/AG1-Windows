"""XTest input only on the owned authenticated GitHub secret fixture display."""
import ctypes as C, os, struct, sys, subprocess, time
from pathlib import Path
work=Path(sys.argv[1]).resolve();mode=sys.argv[2]
assert work.parent.name.startswith('native-candidate-') and work.name.startswith('secret-')
auth=work/'display.auth';data=auth.read_bytes();offset=2;fields=[]
for _ in range(4):
 n=struct.unpack('>H',data[offset:offset+2])[0];offset+=2;fields.append(data[offset:offset+n]);offset+=n
number=int(fields[1]);assert 300<=number<500
os.environ['XAUTHORITY']=str(auth);os.environ['DISPLAY']=f':{number}'
x=C.CDLL('libX11.so.6');xt=C.CDLL('libXtst.so.6')
x.XOpenDisplay.argtypes=[C.c_char_p];x.XOpenDisplay.restype=C.c_void_p
d=x.XOpenDisplay(f':{number}'.encode());assert d
x.XDefaultRootWindow.argtypes=[C.c_void_p];x.XDefaultRootWindow.restype=C.c_ulong
x.XFetchName.argtypes=[C.c_void_p,C.c_ulong,C.POINTER(C.c_void_p)];x.XFetchName.restype=C.c_int
x.XQueryTree.argtypes=[C.c_void_p,C.c_ulong,C.POINTER(C.c_ulong),C.POINTER(C.c_ulong),C.POINTER(C.POINTER(C.c_ulong)),C.POINTER(C.c_uint)]
x.XInternAtom.argtypes=[C.c_void_p,C.c_char_p,C.c_int];x.XInternAtom.restype=C.c_ulong
x.XGetWindowProperty.argtypes=[C.c_void_p,C.c_ulong,C.c_ulong,C.c_long,C.c_long,C.c_int,C.c_ulong,C.POINTER(C.c_ulong),C.POINTER(C.c_int),C.POINTER(C.c_ulong),C.POINTER(C.c_ulong),C.POINTER(C.c_void_p)]
atom=x.XInternAtom(d,b'_NET_WM_NAME',0)
x.XFree.argtypes=[C.c_void_p];x.XCloseDisplay.argtypes=[C.c_void_p]
x.XSetInputFocus.argtypes=[C.c_void_p,C.c_ulong,C.c_int,C.c_ulong]
x.XStringToKeysym.argtypes=[C.c_char_p];x.XStringToKeysym.restype=C.c_ulong
x.XKeysymToKeycode.argtypes=[C.c_void_p,C.c_ulong];x.XKeysymToKeycode.restype=C.c_ubyte
x.XFlush.argtypes=[C.c_void_p];xt.XTestFakeKeyEvent.argtypes=[C.c_void_p,C.c_uint,C.c_int,C.c_ulong]
queue=[x.XDefaultRootWindow(d)];found=None
for _ in range(1000):
 if not queue:break
 window=queue.pop();name=C.c_void_p()
 typ=C.c_ulong();fmt=C.c_int();count=C.c_ulong();remaining=C.c_ulong();value=C.c_void_p()
 if x.XGetWindowProperty(d,window,atom,0,1024,0,0,C.byref(typ),C.byref(fmt),C.byref(count),C.byref(remaining),C.byref(value))==0 and value.value:
  title=C.string_at(value,count.value).decode(errors='replace');x.XFree(value)
  if title=='AG1 — GitHub secret':found=window;break
 if x.XFetchName(d,window,C.byref(name)) and name.value:
  title=C.string_at(name).decode(errors='replace');x.XFree(name)
  if title=='AG1 — GitHub secret':found=window;break
 root=C.c_ulong();parent=C.c_ulong();children=C.POINTER(C.c_ulong)();count=C.c_uint()
 if x.XQueryTree(d,window,C.byref(root),C.byref(parent),C.byref(children),C.byref(count)) and children:
  queue.extend(children[i] for i in range(min(count.value,1000)));x.XFree(children)
try:
 if mode=='present':print('present' if found else 'absent')
 else:
  assert found,'Owned native secret dialog is absent'
  if mode=='screenshot':
   subprocess.run(['import','-display',f':{number}','-window','root',str(work/'native-secret-entry.png')],check=True,timeout=10,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
  else:
   x.XSetInputFocus(d,found,1,0);x.XFlush(d);time.sleep(.05)
   names=list('ownedfixturevalue') if mode=='fill' else ['Return'] if mode=='submit' else ['Escape'] if mode=='cancel' else []
   assert names
   for name in names:
    code=x.XKeysymToKeycode(d,x.XStringToKeysym(name.encode()));assert code
    xt.XTestFakeKeyEvent(d,code,1,0);xt.XTestFakeKeyEvent(d,code,0,0)
   x.XFlush(d);time.sleep(.1)
  print('done')
finally:x.XCloseDisplay(d)
