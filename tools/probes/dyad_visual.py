"""Control the isolated reference, never a user's Dyad profile."""
import json,time
from pathlib import Path
import desktop_visual as native
native.SOCKET=native.ROOT/'.local/p04-dyad/control.sock'
js=native.js;wait=native.wait;click=native.click;screenshot=native.screenshot
ROOT=native.ROOT
def main(script):return native.request({'main':'('+script+')(process.mainModule.require("electron"))'})
def theme(value):
 main('({nativeTheme})=>{nativeTheme.themeSource='+json.dumps(value)+';return true}')
 wait('document.documentElement.classList.contains("'+value+'")')
def pair(name):
 for value in ['light','dark']:
  theme(value);time.sleep(.12);screenshot('dyad-'+name+'-'+value)
