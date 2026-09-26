"""Reopen real pre-fix journal and exercise corrected work intent routing."""
import json,sys,hashlib,traceback
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'tools/release_duration'))
from cli_client import NativeClient
from workload_checks import query,page,records_since
from faults import retain
base=ROOT/'.local/p14-long-horizon/overnight-preferred-pilot-2'
out=ROOT/'.local/p14-long-horizon/overnight-work-followup-1';out.mkdir(mode=0o700)
client=None;result={'status':'running','release_acceptance':False,'duration_run':False}
try:
 client=NativeClient(Path(sys.argv[1]),base/'workspace',base/'state',base/'config.json',out/'client')
 client.ready();before=query(client,'session','history',{'offset':0})['total']
 prompt='A harness regression check is requested in this disposable session. Earlier work observe with kind=DECISION silently stored an observation, not a decision. First deliberately make ONE work observe request with the extra top-level kind=DECISION field and an observation identifying this diagnostic. The updated tool should reject that ambiguous input. Then use work action=record, kind=DECISION and statement="Receipt records apply to their recorded revision, not to subsequently edited source" to create a real decision. Use work show to inspect the actual typed entry. Do not modify files or run shell commands. Report exactly which request failed and what entry was actually stored.'
 receipt=client.execute({'kind':'prompt','input':{'kind':'inline','text':prompt}},seconds=900)
 records=records_since(client,before);retain(out/'records.json',records);retain(out/'task.json',page(client,'runtime','detail_page',{'section':'task'}));retain(out/'context.json',page(client,'runtime','detail_page',{'section':'context'}))
 result.update(status='completed-review-pending',receipt=receipt,prior_journal_entries=before)
except Exception as error:
 result.update(status='failed',error=repr(error));traceback.print_exc();raise
finally:
 if client:
  try:client.close()
  except Exception as error:result.update(status='failed',cleanup_error=repr(error))
 retain(out/'result.json',result)
