"""XTest input only after selecting and verifying the owned AG1 fixture window."""
import ctypes as c,json
from pathlib import Path
root=Path(__file__).resolve().parents[2];process=json.loads((root/'.local/p04-visual/process.json').read_text());pid=process['pid'];assert Path(f'/proc/{pid}/stat').read_text().split(') ')[1].split()[19]==process['start']
x=c.CDLL('libX11.so.6');t=c.CDLL('libXtst.so.6');P=c.c_void_p;U=c.c_ulong
x.XOpenDisplay.argtypes=[c.c_char_p];x.XOpenDisplay.restype=P;d=x.XOpenDisplay(None);assert d
x.XDefaultRootWindow.argtypes=[P];x.XDefaultRootWindow.restype=U;x.XInternAtom.argtypes=[P,c.c_char_p,c.c_int];x.XInternAtom.restype=U
x.XQueryTree.argtypes=[P,U,c.POINTER(U),c.POINTER(U),c.POINTER(c.POINTER(U)),c.POINTER(c.c_uint)]
x.XGetWindowProperty.argtypes=[P,U,U,c.c_long,c.c_long,c.c_int,U,c.POINTER(U),c.POINTER(c.c_int),c.POINTER(U),c.POINTER(U),c.POINTER(c.POINTER(c.c_ubyte))]
x.XFree.argtypes=[P];x.XSetInputFocus.argtypes=[P,U,c.c_int,U];x.XGetInputFocus.argtypes=[P,c.POINTER(U),c.POINTER(c.c_int)];x.XFlush.argtypes=[P];x.XSync.argtypes=[P,c.c_int]
x.XStringToKeysym.argtypes=[c.c_char_p];x.XStringToKeysym.restype=U;x.XKeysymToKeycode.argtypes=[P,U];x.XKeysymToKeycode.restype=c.c_uint;t.XTestFakeKeyEvent.argtypes=[P,c.c_uint,c.c_int,U]
x.XGetGeometry.argtypes=[P,U,c.POINTER(U),c.POINTER(c.c_int),c.POINTER(c.c_int),c.POINTER(c.c_uint),c.POINTER(c.c_uint),c.POINTER(c.c_uint),c.POINTER(c.c_uint)]
atom=x.XInternAtom(d,b'_NET_WM_PID',0)
def owner(w):
 typ=U();fmt=c.c_int();n=U();after=U();data=c.POINTER(c.c_ubyte)();x.XGetWindowProperty(d,w,atom,0,1,0,0,c.byref(typ),c.byref(fmt),c.byref(n),c.byref(after),c.byref(data));value=c.cast(data,c.POINTER(U))[0] if data and n.value else None
 if data:x.XFree(data)
 return value
def find(w,depth=0):
 if owner(w)==pid:
  rr=U();xx=c.c_int();yy=c.c_int();width=c.c_uint();height=c.c_uint();border=c.c_uint();bits=c.c_uint();x.XGetGeometry(d,w,c.byref(rr),c.byref(xx),c.byref(yy),c.byref(width),c.byref(height),c.byref(border),c.byref(bits))
  if width.value>=900 and height.value>=600:return w
 if depth>4:return None
 rr=U();parent=U();children=c.POINTER(U)();n=c.c_uint();x.XQueryTree(d,w,c.byref(rr),c.byref(parent),c.byref(children),c.byref(n));items=list(children[:n.value]) if children else []
 if children:x.XFree(children)
 for child in items:
  found=find(child,depth+1)
  if found:return found
w=find(x.XDefaultRootWindow(d));assert w,'Owned X11 window missing';x.XSetInputFocus(d,w,2,0);x.XSync(d,0);focused=U();revert=c.c_int();x.XGetInputFocus(d,c.byref(focused),c.byref(revert));assert focused.value==w
for character in 'saved ':
 key=x.XKeysymToKeycode(d,x.XStringToKeysym(b'space' if character==' ' else character.encode()));assert key;t.XTestFakeKeyEvent(d,key,1,0);t.XTestFakeKeyEvent(d,key,0,0)
x.XFlush(d);print('Typed fixture text into verified owned AG1 window')
