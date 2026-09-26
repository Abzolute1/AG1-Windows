"""Read-only compact manual review view. Does not decide or publish reviews."""
import json,sys
from pathlib import Path
root=Path(sys.argv[1]);specs=json.loads((root/'native/specs.json').read_text())
verifiers={s['verificationSource'] for s in specs}
for index,d in enumerate(sorted((root/'activities').glob('*-task'))):
 if not (d/'finished.json').exists() or (root/f'claim-reviews/{index:06}.json').exists():continue
 print('\nTASK',index,d.name)
 print('CHECKS',json.dumps(json.loads((d/'verification.json').read_text()),separators=(',',':')))
 print('DIFF', (d/'diff.patch').read_text())
 for row in json.loads((d/'records.json').read_text()):
  m=row.get('message',{});role=m.get('role');blocks=m.get('content',[])
  if not isinstance(blocks,list):continue
  if role=='assistant':
   for b in blocks:
    if b['type']=='toolCall':print('CALL',b.get('name'),json.dumps(b.get('arguments'),ensure_ascii=False,separators=(',',':')))
    elif b['type']=='text':print('FINAL' if m.get('stopReason')=='stop' else 'COMMENT',b['text'])
  elif role=='toolResult':
   name=m.get('toolName');print('RESULT',name,'error='+str(m.get('isError',False)))
   for block_index,b in enumerate(blocks):
    if b['type']!='text':continue
    t=b['text']
    if name=='read' and t in verifiers:print('[Exact unchanged original verifier source]');continue
    if name=='edit' and block_index>0 and not m.get('isError'):continue # exact edit and final diff already shown
    if name=='artifact':
     try:
      v=json.loads(t)
      if 'artifactSearch' in v:
       for match in v['artifactSearch'].get('matches',[]):
        for c in match.get('contexts',[]):print('MATCH',match['id'],c.get('sourceLine'),c.get('matchedText'))
       if not v['artifactSearch'].get('matches'):print(t)
       continue
     except json.JSONDecodeError:pass
     lines=[line for line in t.splitlines() if 'CONTRACT' in line]
     print('\n'.join(lines)[:5000] if lines else t[:750]);continue
    print(t[:5000])
 c=json.loads((d/'context.json').read_text());print('CONTEXT',json.dumps(c.get('stats'),separators=(',',':')))
