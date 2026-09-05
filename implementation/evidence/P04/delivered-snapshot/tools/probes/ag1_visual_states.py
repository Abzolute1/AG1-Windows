"""Matched native fixture states. Requires a just-launched visual-test desktop."""
import json,time
from desktop_visual import *
def theme(value):
 if js('document.documentElement.classList.contains("dark")')!=(value=='dark'):click('Toggle theme')
 wait('document.documentElement.classList.contains("dark")' if value=='dark' else '!document.documentElement.classList.contains("dark")')
def pair(name):
 for value in ['light','dark']:
  theme(value);time.sleep(.1);screenshot('ag1-'+name+'-'+value)
 theme('light')
project=ROOT/'.local/p04-visual/project';(project/'README.md').write_text('# Native desktop fixture\n\nA small project for the matched desktop check.\n')
wait('!!document.querySelector(".home")');pair('home');js('document.querySelector(".open-card").click();true');wait('!!document.querySelector("[role=dialog]")');pair('dialog');click('Cancel')
js('document.querySelector(".recent button").click();true');wait('!!document.querySelector(".chat-layout")');click('New conversation');wait('document.body.innerText.includes("Ready when you are.")')
text('textarea[aria-label="Message AG1"]','[slow] Review this project');wait('![...document.querySelectorAll("button")].find(b=>b.getAttribute("aria-label")==="Send message").disabled');click('Send message');wait('[...document.querySelectorAll("button")].some(b=>b.getAttribute("aria-label")==="Stop generation")');pair('loading');wait('document.querySelector("footer").innerText.includes("All caught up")');pair('chat')
click('Files');wait('[...document.querySelectorAll(".file-tree button")].some(b=>b.textContent==="README.md")');click('README.md');wait('!!document.querySelector(".monaco-editor textarea")')
js('document.querySelector(".monaco-editor textarea").focus();document.execCommand("insertText",false,"Pending edit: ");true');wait('!!document.querySelector(".dirty")');pair('files-dirty')
external='# External editor change\n';(project/'README.md').write_text(external);click('Save');wait('!!document.querySelector(".error[role=alert]")');assert (project/'README.md').read_text()==external;pair('error');click('Discard');click('Dismiss error');click('Changes');wait('document.querySelector(".diff-content").textContent.includes("External editor change")');pair('diff')
click('Settings');wait('document.querySelectorAll(".model-card").length===3');wait('[...document.querySelectorAll(".model-card button")].every(b=>!b.disabled)');pair('settings')
viewport=js('({width:innerWidth,height:innerHeight,scale:devicePixelRatio})');assert viewport=={'width':1440,'height':900,'scale':1},viewport
(ROOT/'implementation/evidence/P04/ag1-visual-states.json').write_text(json.dumps({'states':['home','chat','files-dirty','diff','settings','dialog','loading','error'],'themes':['light','dark'],'viewport':viewport,'native_save_conflict_preserved_external_bytes':True,'fixture_provider':True},indent=2))
print('Captured all eight native states in both themes; save conflict preserved external bytes.')
