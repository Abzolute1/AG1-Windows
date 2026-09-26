import importlib.util,json,time,urllib.request
from pathlib import Path
p=Path('.local/free-search-recovery-20260914');spec=importlib.util.spec_from_file_location('probe',p/'you-probe.py');m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)
queries=[
 ('rust_official','site:doc.rust-lang.org ownership borrowing',None),
 ('rust_twenty','Rust ownership borrowing documentation',None),
 ('oslo_official','site:ruter.no tickets prices Oslo',None),
 ('quoted_control','"zxqv719824610ag1searchcontrol"',None),
]
reports=[]
for i,(name,query,freshness) in enumerate(queries,10):
 args={'query':query,'count':20 if name=='rust_twenty' else 5,'extraction':'highlights'}
 if freshness:args['freshness']=freshness
 body={'jsonrpc':'2.0','id':i,'method':'tools/call','params':{'name':'you-search','arguments':args}}
 start=time.monotonic()
 try:
  with m.opener.open(urllib.request.Request('https://api.you.com/mcp?profile=free',data=json.dumps(body).encode(),headers=m.headers),timeout=25) as r:
   raw=r.read(1000000).decode();(p/f'you-query-{name}.txt').write_text(raw)
   parsed=json.loads(raw) if raw.lstrip().startswith('{') else [json.loads(x[5:].strip()) for x in raw.splitlines() if x.startswith('data:')][-1]
   (p/f'you-query-{name}.json').write_text(json.dumps(parsed,indent=2)+'\n')
   result=parsed.get('result',{});structured=result.get('structuredContent')
   if structured is None:
    for block in result.get('content',[]):
     if block.get('type')=='text':
      try:structured=json.loads(block['text']);break
      except ValueError:pass
   report={'name':name,'query':query,'freshness':freshness,'seconds':round(time.monotonic()-start,3),'httpStatus':r.status,'headers':dict(r.headers),'isError':result.get('isError',False),'rpcError':parsed.get('error'),'data':structured,'text':str(result.get('content',[]))[:250] if not structured else None}
 except Exception as e:report={'name':name,'error':str(e),'seconds':round(time.monotonic()-start,3)}
 reports.append(report)
 data=report.get('data') or {};rows=(data.get('results') or {}).get('web',[])
 print(json.dumps({'name':name,'seconds':report['seconds'],'error':report.get('error') or report.get('rpcError') or report.get('isError'),'webCount':len(rows),'top':[{k:x.get(k) for k in ['title','url','description','snippets','page_age']} for x in rows[:2]],'text':report.get('text')}),flush=True)
 if report.get('error') or report.get('rpcError') or report.get('isError'):break
(p/'you-quality-followup.json').write_text(json.dumps(reports,indent=2)+'\n')
