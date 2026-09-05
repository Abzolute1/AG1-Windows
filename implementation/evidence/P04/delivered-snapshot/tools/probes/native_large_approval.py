"""The actual dialog must expose every review page before enabling a large approval."""
import json
from desktop_visual import *
click('Chat');wait('document.querySelector("footer").innerText.includes("All caught up")');text('textarea[aria-label="Message AG1"]','[large-approval] Review the complete request');click('Send message');wait('!!document.querySelector(".approval-dialog")')
allow='[...document.querySelectorAll("button")].find(b=>b.textContent.trim()==="Allow this operation")'
assert js(allow+'.disabled');pages=0
while js(allow+'.disabled'):
 label='Next request page' if pages else 'Read complete request';click(label);pages+=1
 wait('!document.querySelector("footer").innerText.includes("Working…")')
 if pages>10:raise RuntimeError('Review pagination did not finish')
assert pages>=2
click('Deny');wait('document.querySelector(".approval-dialog")===null');wait('document.querySelector("footer").innerText.includes("All caught up")');click('New conversation');wait('document.body.innerText.includes("Ready when you are.")')
(ROOT/'implementation/evidence/P04/native-large-approval.json').write_text(json.dumps({'pages':pages,'allow_disabled_until_final_page':True,'denied_after_review':True},indent=2));print('Large request review paged correctly; returned AG1 to a fresh conversation.')
