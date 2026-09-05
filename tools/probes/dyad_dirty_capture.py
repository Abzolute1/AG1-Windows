"""Recapture a loaded reference editor; assert visibility before retaining evidence."""
import json, shutil
from dyad_visual import ROOT, js, wait, theme, screenshot

wait('!!window.monaco?.editor.getEditors().length')
js('''(()=>{
 const editor=window.monaco.editor.getEditors().find(e=>e.getModel()?.uri.path.endsWith("README.md"));
 if(!editor)throw Error("README editor unavailable");
 editor.focus();
 editor.executeEdits("fixture",[{range:new window.monaco.Range(1,1,1,1),text:"Unsaved fixture: "}]);
 return true;
})()''')
for value in ['light','dark']:
 theme(value)
 wait('''(()=>{
  const editor=document.querySelector('.monaco-editor');
  const rect=editor?.getBoundingClientRect();
  const save=document.querySelector('[data-testid=save-file-button]');
  return rect?.width>200 && rect.height>200 && save && !save.disabled;
 })()''')
 name='dyad-files-dirty-'+value
 screenshot(name)
 shutil.copyfile(ROOT/'.local/p04-dyad'/f'{name}.png',ROOT/'implementation/evidence/P04/screenshots'/f'{name}.png')
print(json.dumps({'editor_visible':True,'unsaved':True,'themes':['light','dark']}))
