import collections,hashlib,json,re,sqlite3
from pathlib import Path
ROOT=Path('/home/alex/Projects/AG1');E=ROOT/'implementation/evidence/ag1-optimized-validation-20260914';W=ROOT/'.local/p14-long-horizon/ag1perf-20260914'
rows=[]
def answer(v):
 a=[m for m in v['details']['messages'] if m['role']=='assistant'];return '\n'.join(b.get('text','') for b in a[-1]['blocks'] if b['kind']=='text')
for arm,label in [('original','original'),('candidate','candidate-verified')]:
 o=E/label;r=json.loads((o/'result.json').read_text());results={}
 for task in ['chat','edit','web']:
  v=json.loads((o/(task+'-view.json')).read_text());text=answer(v);(o/(task+'-answer.md')).write_text(text+'\n')
  db=sqlite3.connect('file:'+str(W/label/'state/store/storage.sqlite')+'?mode=ro',uri=True)
  journal=[json.loads(raw) for raw, in db.execute('select normalized from entries where session=? order by ordinal',(v['snapshot']['scope']['session'],))];db.close()
  prefix={'chat':'Answer directly without using tools.','edit':'In the attached project, read services.json.','web':'Use webSearch once to find Python documentation'}[task]
  def content(message):
   val=message.get('content',[])
   return val if isinstance(val,str) else '\n'.join(x.get('text','') for x in val if x.get('type')=='text')
  messages=[x['message'] for x in journal if x.get('type')=='message']
  first=next(i for i,x in enumerate(messages) if x.get('role')=='user' and content(x).startswith(prefix))
  last=next((i for i in range(first+1,len(messages)) if messages[i].get('role')=='user'),len(messages));messages=messages[first:last]
  (o/(task+'-messages.json')).write_text(json.dumps(messages,indent=2)+'\n')
  tools=[{'name':m.get('toolName'),'failed':bool(m.get('isError'))} for m in messages if m.get('role')=='toolResult']
  counts=dict(collections.Counter(x['name'] for x in tools))
  results[task]={'answer':text,'whitespace_word_count':len(text.split()),'journal_tool_result_counts':counts,'failed_tools':[x['name'] for x in tools if x['failed']],'error':v['error'],'native_terminal':v['details']['activity'].get('last_terminal'),'task_status':v['details']['task'].get('status'),'latest_assistant_usage':v['details']['activity'].get('latest_assistant_usage')}
  if task=='chat':results[task]['exact_answer_pass']=json.loads(text)=={'answer':323}
  if task=='web':
   results[task]['web_tool_execution_pass']=counts.get('webSearch')==1 and counts.get('webFetch')==1 and not any(x['failed'] for x in tools)
   results[task]['under_100_words']=len(text.split())<=100
   results[task]['official_citation_present']='docs.python.org/3/library/time.html' in text
 before=(o/'services-before.json').read_text();expected=before.replace('"svc-047": {\n    "timeout_ms": 450,','"svc-047": {\n    "timeout_ms": 875,');actual=(W/label/'workspace/services.json').read_text();assert expected!=before
 r['independent_edit_exact_pass']=actual==expected;r['scored_tasks']=results
 rows.append({'arm':arm,'label':label,'report':r});print(arm,{k:{q:x[q] for q in x if q not in ['answer','latest_assistant_usage','native_terminal']} for k,x in results.items()},flush=True)
(E/'scored-results.json').write_text(json.dumps({'note':'Two full desktop sequences; one per arm. Diagnostic end-to-end timings, not a statistical model benchmark. Native views are bounded; negative message-count deltas in the initial driver and first-assistant-block timing are not reliable token metrics.','runs':rows},indent=2)+'\n')
