"""Compact evidence view for manual Codex claim review; never approves a task."""
import json
from pathlib import Path
import sys
root=Path(sys.argv[1])
for index,d in enumerate(sorted((root/'activities').glob('*-task'))):
    if not (d/'finished.json').exists() or (root/f'claim-reviews/{index:06}.json').exists():
        continue
    print('\nTASK',index,str(d))
    print((d/'verification.json').read_text())
    print((d/'diff.patch').read_text())
    for row in json.loads((d/'records.json').read_text()):
        m=row.get('message',{});role=m.get('role')
        if role=='assistant':
            for b in m.get('content',[]):
                if b['type']=='toolCall':print('CALL',b.get('name'),b.get('arguments'))
                elif b['type']=='text':print('FINAL' if m.get('stopReason')=='stop' else 'COMMENT',b['text'])
        elif role=='toolResult':
            for b in m.get('content',[]):
                if b['type']=='text':
                    t=b['text']
                    if m['toolName']=='artifact':
                        lines=[l for l in t.splitlines() if 'CONTRACT' in l]
                        shown='\n'.join(lines)[:3000] if lines else t[:700]
                    else:shown=t[:3000]
                    print('RESULT',m['toolName'],'error='+str(m.get('isError',False)),shown)
    c=json.loads((d/'context.json').read_text())
    print('CONTEXT',json.dumps(c.get('stats')))
