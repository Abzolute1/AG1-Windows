import urllib.request,json,time
from pathlib import Path
out=Path('.local/free-search-recovery-20260914');opener=urllib.request.build_opener(urllib.request.ProxyHandler({}))
headers={'Accept':'application/json, text/event-stream','Content-Type':'application/json','User-Agent':'Mozilla/5.0','MCP-Protocol-Version':'2025-03-26'}
def rpc(method,params=None,id=1):
 body={'jsonrpc':'2.0','id':id,'method':method}
 if params is not None:body['params']=params
 start=time.monotonic()
 try:
  response=opener.open(urllib.request.Request('https://api.you.com/mcp?profile=free',data=json.dumps(body).encode(),headers=headers),timeout=25)
 except urllib.error.HTTPError as e:
  (out/f'you-error-{id}.json').write_text(json.dumps({'status':e.code,'headers':dict(e.headers),'body':e.read(50000).decode(errors='replace')},indent=2));raise
 with response as r:
  text=r.read(1000000).decode();(out/f'you-{id}.txt').write_text(text)
  if r.headers.get('Mcp-Session-Id'):headers['Mcp-Session-Id']=r.headers['Mcp-Session-Id']
  result=json.loads(text) if text.lstrip().startswith('{') else [json.loads(line[5:].strip()) for line in text.splitlines() if line.startswith('data:')][-1]
  (out/f'you-{id}.json').write_text(json.dumps(result,indent=2)+'\n')
  print(method,round(time.monotonic()-start,3),json.dumps(result)[:6500],flush=True)
  return result
if __name__=='__main__':
 rpc('initialize',{'protocolVersion':'2025-03-26','capabilities':{},'clientInfo':{'name':'AG1-search-evaluation','version':'1.0'}},1)
 rpc('tools/list',{},2)
