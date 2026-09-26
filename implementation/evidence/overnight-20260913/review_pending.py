"""Read-only view of unreviewed duration evidence; makes no quality decisions."""
import collections, difflib, json, sys, time
from pathlib import Path
root=Path(sys.argv[1]).resolve()
arguments=[a for a in sys.argv[2:] if a!='--compact']
maximum=int(arguments[0]) if arguments else 3
compact='--compact' in sys.argv[2:]
def visible(records, omit_final_text=False):
    lines=[]
    final_index=next((i for i in range(len(records)-1,-1,-1)
                      if records[i].get('message',{}).get('role')=='assistant'),-1)
    for row_index,row in enumerate(records):
        if row.get('type')=='custom':lines.append('CUSTOM '+str(row.get('customType')))
        message=row.get('message',{})
        if message.get('role')=='user':continue
        content=message.get('content',[])
        if isinstance(content,str):
            if not (omit_final_text and row_index==final_index):
                lines.append(str(message.get('role'))+' '+content)
        elif isinstance(content,list):
            for block in content:
                if block.get('type')=='thinking':continue
                if omit_final_text and row_index==final_index and block.get('type')=='text':continue
                block=dict(block)
                if block.get('type')=='toolCall':block['id']='<call-id>'
                lines.append(str(message.get('role'))+' '+str(message.get('toolName'))+' '+json.dumps(block,ensure_ascii=False))
    return lines
start=json.loads((root/'started.json').read_text())
tasks=sorted((root/'activities').glob('*-task'))
print('ELAPSED_MINUTES',round((time.monotonic_ns()-start['start_monotonic_ns'])/6e10,2))
print('TASKS',sum((p/'finished.json').exists() for p in tasks),'REVIEWS',len(list((root/'claim-reviews').glob('*.json'))))
if (root/'duration-result.json').exists():
    result=json.loads((root/'duration-result.json').read_text())
    print('DURATION_RESULT',result['status'],result.get('error'))
shown=0
for index,directory in enumerate(tasks):
    if (root/f'claim-reviews/{index:06}.json').exists() or not (directory/'finished.json').exists():
        continue
    if shown>=maximum:break
    shown+=1
    print('\nTASK',index,directory.name)
    records=json.loads((directory/'records.json').read_text())
    verification=json.loads((directory/'verification.json').read_text())
    previous=None
    if compact:
        for old_index in range(index-1,-1,-1):
            old=tasks[old_index]
            if ((root/f'claim-reviews/{old_index:06}.json').exists()
                    and (old/'verification.json').exists()
                    and json.loads((old/'verification.json').read_text())['id']==verification['id']):
                previous=(old_index,old);break
    if previous:
        old_index,old=previous
        status=json.loads((root/f'claim-reviews/{old_index:06}.json').read_text())['status']
        print('EXACT_VISIBLE_COMPARISON_BASELINE',old_index,status,
              '(only tool-call IDs normalized; user prompts/thinking omitted; final text printed fully below)')
        prior=visible(json.loads((old/'records.json').read_text()),True)
        print('\n'.join(difflib.unified_diff(prior,visible(records,True),n=0,lineterm='')) or 'IDENTICAL_VISIBLE_EXECUTION')
        calls=[b.get('name') for row in records for b in row.get('message',{}).get('content',[])
               if isinstance(b,dict) and b.get('type')=='toolCall']
        print('CURRENT_TOOL_CALL_COUNTS',dict(collections.Counter(calls)))
        last=next((row['message'] for row in reversed(records) if row.get('message',{}).get('role')=='assistant'),{})
        print('CURRENT_FINAL_TEXT','\n'.join(b['text'] for b in last.get('content',[]) if isinstance(b,dict) and b.get('type')=='text'))
    for row in ([] if previous else records):
        if row.get('type')=='custom':print('CUSTOM',row.get('customType'))
        message=row.get('message',{})
        if message.get('role')=='user':continue
        content=message.get('content',[])
        if isinstance(content,str):print(message.get('role'),content)
        elif isinstance(content,list):
            for block in content:
                if block.get('type')!='thinking':
                    print(message.get('role'),message.get('toolName'),json.dumps(block,ensure_ascii=False))
    print('DIFF',(directory/'diff.patch').read_text())
    print('EXTERNAL_VERIFIER',(directory/'verifier.log').read_text())
    print('VERIFICATION',(directory/'verification.json').read_text())
