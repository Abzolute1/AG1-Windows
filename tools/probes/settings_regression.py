"""Exercise the reported stale Settings revision through the actual native UI."""
import json,time
from pathlib import Path
from desktop_visual import js,wait,click,async_js,screenshot,ROOT
wait('!!document.querySelector(".recent button")')
js('document.querySelector(".recent button").click();true')
wait('document.body.innerText.includes("Fixture provider")')
for i in range(10):
 click('Settings');wait('document.querySelectorAll(".model-card").length===3');time.sleep(.08)
 click('Changes');time.sleep(.08)
click('Settings');wait('document.querySelectorAll(".model-card").length===3')
for i in [1,2,0]:
 wait('[...document.querySelectorAll(".model-card button")].every(b=>!b.disabled)')
 js(f'document.querySelectorAll(".model-card button")[{i}].click();true')
 wait(f'document.querySelectorAll(".model-card button")[{i}].textContent.includes("Selected")')
assert js('!document.querySelector(".error[role=alert]")'),js('document.querySelector(".error[role=alert]").textContent')
snap=async_js('return await window.__TAURI_INTERNALS__.invoke("bridge_view")')
# Reopen in the same host as well: each worker launch needs a fresh evidence spool.
click('Home');js('document.querySelector(".recent button").click();true')
wait('document.querySelector(".chat-layout")!==null && !document.querySelector(".error[role=alert]")')
again=async_js('return await window.__TAURI_INTERNALS__.invoke("bridge_view")')
assert snap['snapshot']['scope']['instance']!=again['snapshot']['scope']['instance']
assert snap['snapshot']['scope']['session']==again['snapshot']['scope']['session']
click('Settings');wait('document.querySelectorAll(".model-card").length===3')
evidence={'scenario':'Repeated Settings/Changes navigation and all three profile selections through actual native React UI, then workspace reopen','visits':12,'model_selections':3,'alert':None,'snapshot':again,'viewport':js('({width:innerWidth,height:innerHeight,scale:devicePixelRatio})'),'screenshot':screenshot('ag1-settings-fixed-light')}
(ROOT/'implementation/evidence/P04/settings-regression.json').write_text(json.dumps(evidence,indent=2))
print(json.dumps({'passed':True,'model':again['snapshot']['model'],'screenshot':evidence['screenshot']}))
