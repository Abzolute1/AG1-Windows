"""Native WebKit test broker client; never reads another application's window."""
import json,socket,time
from pathlib import Path
ROOT=Path(__file__).resolve().parents[2]
SOCKET=ROOT/'.local/p04-visual/control.sock'
def request(value):
 with socket.socket(socket.AF_UNIX) as s:
  s.settimeout(20);s.connect(str(SOCKET));s.sendall((json.dumps(value)+'\n').encode());reply=json.loads(s.makefile().readline())
  if 'error' in reply:raise RuntimeError(reply['error'])
  return reply.get('ok')
def js(script):return request({'script':script})
def wait(script,timeout=15):
 end=time.monotonic()+timeout
 while time.monotonic()<end:
  result=js(script)
  if result:return result
  time.sleep(.05)
 raise RuntimeError('Native visual condition timed out: '+script)
def async_js(script):
 js('window.__visualResult=undefined;window.__visualError=undefined;(async()=>{'+script+'})().then(x=>window.__visualResult={value:x??null},e=>window.__visualError=String(e));true')
 wait('!!window.__visualResult || !!window.__visualError',30)
 if js('window.__visualError??null'):raise RuntimeError(js('window.__visualError'))
 return js('window.__visualResult.value')
def click(label):
 lookup=f'const buttons=[...document.querySelectorAll("button")];const b=buttons.find(b=>b.getAttribute("aria-label")==={json.dumps(label)})??buttons.find(b=>b.textContent.trim().toLowerCase()==={json.dumps(label.lower())});'
 wait('(()=>{'+lookup+'return !!b&&!b.disabled;})()')
 return js('(()=>{'+lookup+'b.click();return true;})()')

def text(selector,value):return js(f'(()=>{{const node=document.querySelector({json.dumps(selector)});if(!node)throw Error("Input missing");Object.getOwnPropertyDescriptor(node.tagName==="TEXTAREA"?HTMLTextAreaElement.prototype:HTMLInputElement.prototype,"value").set.call(node,{json.dumps(value)});node.dispatchEvent(new Event("input",{{bubbles:true}}));return true;}})()')
def screenshot(name):return request({'screenshot':name+'.png'})
if __name__=='__main__':print(json.dumps(js('({title:document.title,text:document.body.innerText.slice(0,2000),width:innerWidth,height:innerHeight,dpr:devicePixelRatio,tauri:!!window.__TAURI_INTERNALS__})'),indent=2))
