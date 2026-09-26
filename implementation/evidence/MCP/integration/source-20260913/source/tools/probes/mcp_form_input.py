"""Synthetic GTK input on an authenticated, owned MCP fixture X server only.

No credential value is accepted in argv or returned on stdout. The fixture value
is read from its private file and typed directly into GTK using XTest.
"""
import ctypes as c
import ctypes.util
import json
import os
from pathlib import Path
import struct
import subprocess
import sys
import time

work = Path(sys.argv[1]).resolve()
assert str(work).startswith('/home/alex/Projects/AG1/.local/p14-live-fixes/native-candidate-mcp-c3-')
action = sys.argv[2]
target = sys.argv[3]
assert target == 'form'
title='AG1 — MCP needs your input'
assert action in ('status', 'cancel', 'keys', 'click', 'answer', 'continue', 'screenshot')
auth = work / 'display.auth'
data = auth.read_bytes()
offset = 2
fields = []
for _ in range(4):
    length = struct.unpack('>H', data[offset:offset+2])[0]
    offset += 2
    fields.append(data[offset:offset+length])
    offset += length
display = int(fields[1])
assert 300 <= display < 500
os.environ['XAUTHORITY'] = str(auth)
x = c.CDLL(ctypes.util.find_library('X11'))
xt = c.CDLL(ctypes.util.find_library('Xtst'))
x.XOpenDisplay.argtypes = [c.c_char_p]
x.XOpenDisplay.restype = c.c_void_p
x.XDefaultRootWindow.argtypes = [c.c_void_p]
x.XDefaultRootWindow.restype = c.c_ulong
x.XQueryTree.argtypes = [c.c_void_p,c.c_ulong,c.POINTER(c.c_ulong),c.POINTER(c.c_ulong),c.POINTER(c.POINTER(c.c_ulong)),c.POINTER(c.c_uint)]
x.XFetchName.argtypes = [c.c_void_p,c.c_ulong,c.POINTER(c.c_void_p)]
x.XInternAtom.argtypes = [c.c_void_p,c.c_char_p,c.c_int]
x.XInternAtom.restype = c.c_ulong
x.XGetWindowProperty.argtypes = [c.c_void_p,c.c_ulong,c.c_ulong,c.c_long,c.c_long,c.c_int,c.c_ulong,c.POINTER(c.c_ulong),c.POINTER(c.c_int),c.POINTER(c.c_ulong),c.POINTER(c.c_ulong),c.POINTER(c.c_void_p)]
x.XFree.argtypes = [c.c_void_p]
x.XSetInputFocus.argtypes = [c.c_void_p,c.c_ulong,c.c_int,c.c_ulong]
x.XGetInputFocus.argtypes = [c.c_void_p,c.POINTER(c.c_ulong),c.POINTER(c.c_int)]
x.XRaiseWindow.argtypes = [c.c_void_p,c.c_ulong]
x.XKeysymToKeycode.argtypes = [c.c_void_p,c.c_ulong]
x.XKeysymToKeycode.restype = c.c_uint
x.XFlush.argtypes = [c.c_void_p]
x.XCloseDisplay.argtypes = [c.c_void_p]
xt.XTestFakeKeyEvent.argtypes = [c.c_void_p,c.c_uint,c.c_int,c.c_ulong]
d = x.XOpenDisplay(f':{display}'.encode())
assert d, 'Owned display unavailable'
try:
    root = x.XDefaultRootWindow(d)
    title_atom = x.XInternAtom(d,b'_NET_WM_NAME',0)
    def find_dialog(window, depth=0):
        assert depth < 16
        # GTK uses the UTF-8 EWMH title for this Unicode window name.
        # WM_NAME/XFetchName alone need not expose that title.
        actual_type, actual_format = c.c_ulong(), c.c_int()
        items, remaining, value = c.c_ulong(), c.c_ulong(), c.c_void_p()
        if x.XGetWindowProperty(d,window,title_atom,0,1024,0,0,c.byref(actual_type),c.byref(actual_format),c.byref(items),c.byref(remaining),c.byref(value)) == 0 and value:
            try:
                if actual_format.value == 8 and c.string_at(value,items.value).decode('utf-8',errors='replace') == title:
                    return window
            finally:
                x.XFree(value)
        name = c.c_void_p()
        if x.XFetchName(d,window,c.byref(name)) and name:
            try:
                if 'MCP private values' in c.string_at(name).decode('utf-8',errors='replace'):
                    return window
            finally:
                x.XFree(name)
        root_return,parent,children,count = c.c_ulong(),c.c_ulong(),c.POINTER(c.c_ulong)(),c.c_uint()
        if x.XQueryTree(d,window,c.byref(root_return),c.byref(parent),c.byref(children),c.byref(count)):
            try:
                assert count.value <= 1024
                for i in range(count.value):
                    found = find_dialog(children[i],depth+1)
                    if found:
                        return found
            finally:
                if children:
                    x.XFree(children)
        return None
    window = find_dialog(root)
    if action == 'status':
        print(json.dumps({'dialog':bool(window)}))
    elif action == 'screenshot':
        name = sys.argv[4]
        assert Path(name).name == name and name.endswith('.png')
        subprocess.run(['import','-display',f':{display}','-window','root',str(work/name)],check=True,timeout=20)
        print(json.dumps({'screenshot':name}))
    else:
        assert window, 'Private dialog not open'
        x.XRaiseWindow(d,window)
        x.XSetInputFocus(d,window,2,0)
        focused, revert = c.c_ulong(), c.c_int()
        x.XGetInputFocus(d,c.byref(focused),c.byref(revert))
        assert focused.value == window, 'Private dialog did not receive focus'
        def key(symbol):
            code=x.XKeysymToKeycode(d,symbol)
            assert code
            assert xt.XTestFakeKeyEvent(d,code,1,0)
            assert xt.XTestFakeKeyEvent(d,code,0,0)
        if action == 'answer':
            # Positions established from this exact native narrow form screenshot.
            xt.XTestFakeMotionEvent.argtypes=[c.c_void_p,c.c_int,c.c_int,c.c_int,c.c_ulong]
            xt.XTestFakeButtonEvent.argtypes=[c.c_void_p,c.c_uint,c.c_int,c.c_ulong]
            def click(px,py):
                assert xt.XTestFakeMotionEvent(d,-1,px,py,0)
                assert xt.XTestFakeButtonEvent(d,1,1,0)
                assert xt.XTestFakeButtonEvent(d,1,0,0)
                x.XFlush(d);time.sleep(.15)
            click(400,243);key(0xff54);key(0xff0d);x.XFlush(d);time.sleep(.15)
            click(101,524);click(700,565)
        elif action == 'click':
            px,py=map(int,sys.argv[4:6]);assert 0<=px<1440 and 0<=py<900
            xt.XTestFakeMotionEvent.argtypes=[c.c_void_p,c.c_int,c.c_int,c.c_int,c.c_ulong]
            xt.XTestFakeButtonEvent.argtypes=[c.c_void_p,c.c_uint,c.c_int,c.c_ulong]
            assert xt.XTestFakeMotionEvent(d,-1,px,py,0)
            assert xt.XTestFakeButtonEvent(d,1,1,0)
            assert xt.XTestFakeButtonEvent(d,1,0,0)
        elif action == 'keys':
            symbols={'Tab':0xff09,'Return':0xff0d,'Down':0xff54,'Up':0xff52,'Space':0x20,'Escape':0xff1b}
            for name in sys.argv[4:]:
                key(symbols[name]);x.XFlush(d);time.sleep(.1)
        elif action == 'continue':
            key(0xff0d)
        else:
            key(0xff1b)  # Escape cancels GTK's modal dialog.
        x.XFlush(d)
        print(json.dumps({'input':action,'target':'owned GTK dialog'}))
finally:
    x.XCloseDisplay(d)
