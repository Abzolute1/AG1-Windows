import json,hashlib,sqlite3,zlib
from pathlib import Path
r=Path.cwd();e=r/'implementation/evidence/scenario-validation-20260913';d=r/'.local/p14-long-horizon/scenario-validation-20260913/candidate';sha=lambda b:hashlib.sha256(b).hexdigest();canonical=lambda x:json.dumps(x,sort_keys=True,separators=(',',':')).encode();rows=[];systems=[];prefixes=[]
for i,cap in [(0,7),(1,5)]:
 result=json.loads((d/f'task-{i}-result.json').read_text());records=json.loads((d/f'task-{i}-records.json').read_text());requests=json.loads((d/f'task-{i}-requests.json').read_text());name=result['verifier'];calls=result['tool_calls'];system=requests[0]['request']['systemPrompt'];systems.append(system);prefix={k:requests[0]['request'][k] for k in ['systemPrompt','tools','model','thinking']};prefixes.append(prefix)
 contracts=[]
 for req in requests:
  texts=[b.get('text','') for m in req['request']['messages'] for b in m.get('content',[]) if isinstance(b,dict)]
  current=[t for t in texts if t.startswith('<runtime-verification-contract>')]
  assert len(current)==1 and f'node --test {name}' in current[0]
  contracts.append(sha(current[0].encode()))
 assert '<runtime-verification-contract>' not in json.dumps(records)
 verifier_calls=[c for c in calls if c['name']=='bash' and '--test' in c['arguments'].get('command','')];assert len(verifier_calls)==1 and verifier_calls[0]['arguments']['command']==f'node --test {name}'
 assert result['verifier_exit']==0 and result['changed_files']==['normalize.mjs'] and result['receipt']['terminal']['operation']['status']=='completed'
 original=[['4 sec',min(4,cap)],['99',cap],['bad',0],['bad4',0],['-2',0],['0',0],['',0],['Infinity',0],['4.9',min(4,cap)],['  +2tail',2],['1e3',1],['0x10',0]]
 expected='import test from "node:test"; import assert from "node:assert/strict"; import {normalize} from "./normalize.mjs";\ntest("current string normalization contract",()=>{ for(const [input,expected] of '+json.dumps(original)+') assert.equal(normalize(input),expected,JSON.stringify(input)); });\n'
 assert (d/'workspace'/name).read_bytes()==expected.encode()
 final=[b['text'] for row in records if (m:=row.get('message',{})).get('role')=='assistant' and m.get('stopReason')=='stop' for b in m['content'] if b.get('type')=='text']
 row={'task':i,'cap':cap,'status':'PASS','verifier':name,'original_verifier_sha256':sha(expected.encode()),'native_command':'completed','original_assertions':12,'node_test_groups':1,'verifier_calls':len(verifier_calls),'changed_files':result['changed_files'],'seconds':(result['receipt']['finished_monotonic_ns']-result['receipt']['started_monotonic_ns'])/1e9,'first_request_system_sha256':sha(system.encode()),'first_request_prefix_sha256':sha(canonical(prefix)),'first_request_usage':result['usage'][0],'request_count':len(requests),'one_current_transient_contract_each_request':True,'contract_absent_from_journal_records':True,'final_answer':final,'manual_claim_review':'PASS: source, cap, all 12 listed values and observed 1 test group pass agree with original verifier and completed receipt.'};rows.append(row)
assert systems[0]==systems[1] and prefixes[0]==prefixes[1]
db=sqlite3.connect('file:'+str(d/'state/store/storage.sqlite')+'?mode=ro',uri=True);artifacts=[json.loads(x[0]) for x in db.execute('SELECT record FROM artifacts')];db.close();timings=[]
for i in [0,1]:
 rs=json.loads((d/f'task-{i}-records.json').read_text());bash=[x for x in rs if x.get('message',{}).get('toolName')=='bash'];assert len(bash)==1
 msg=bash[0];art=next(a for a in artifacts if a['id']==msg['message']['details']['rawOutputArtifact']['id']);assert art['createdAt']==msg['timestamp']
 timings.append({'task':i,'artifact':art['id'],'artifact_createdAt':art['createdAt'],'turn_timestamp':msg['timestamp'],'tool_result_execution_timestamp_ms':msg['message']['timestamp']})
assert timings[1]['artifact_createdAt']>timings[0]['artifact_createdAt']
out={'tasks':rows,'stable_system_and_prefix':True,'timestamp_review':timings,'timestamp_scope':'Artifact metadata uses its actual turn timestamp; tool-result message also records execution time. Later turns no longer inherit the first prompt timestamp.','historical_cache_comparison':{'old_task_2_cached_tokens':0,'old_system_first_difference_character':2336,'previous_fixed_sample_cached_tokens':4424,'new_fixed_sample_cached_tokens':rows[1]['first_request_usage']['cacheRead'],'threshold_or_latency_claim':False}}
(e/'live-audit.json').write_text(json.dumps(out,indent=2)+'\n');print(json.dumps({'repairs':'PASS','stable_system_and_prefix':True,'cached_tokens_task_2':rows[1]['first_request_usage']['cacheRead'],'timestamps':'PASS'},indent=2))
