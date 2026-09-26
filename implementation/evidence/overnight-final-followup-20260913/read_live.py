import json,hashlib
from pathlib import Path
D=Path('.local/p14-long-horizon/fix-validation-20260913-v4/candidate')
for p in sorted(D.glob('task-*-result.json')):
 r=json.loads(p.read_text());records=json.loads(p.with_name(p.name.replace('result','records')).read_text());requests=json.loads(p.with_name(p.name.replace('result','requests')).read_text())
 print('\nTASK',r['index'],json.dumps({k:r[k] for k in ['receipt','verifier_exit','changed_files','source','request_count']},ensure_ascii=False))
 print('FIRST USAGE',r['usage'][0]);print('SYSTEM',hashlib.sha256(requests[0]['request']['systemPrompt'].encode()).hexdigest())
 for e in records:
  m=e.get('message',{})
  if m.get('role')=='assistant' and m.get('stopReason')=='stop':print('FINAL',json.dumps([b.get('text') for b in m.get('content',[]) if b.get('type')=='text'],ensure_ascii=False))
  if m.get('role')=='custom':print('CUSTOM',m.get('customType'),m.get('content'))
 print('CALLS',json.dumps(r['tool_calls'],ensure_ascii=False))
