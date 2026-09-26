import urllib.request,urllib.parse,json,time,hashlib
from pathlib import Path
out=Path('.local/free-search-recovery-20260914')
opener=urllib.request.build_opener(urllib.request.ProxyHandler({}))
headers={'User-Agent':'Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/146.0.0.0 Safari/537.36','Accept':'text/html,application/xhtml+xml','Accept-Language':'en-US,en;q=0.9','Content-Type':'application/x-www-form-urlencoded','Sec-Fetch-Dest':'document','Sec-Fetch-Mode':'navigate','Sec-Fetch-Site':'same-origin','Sec-Fetch-User':'?1'}
reports=[]
for name,endpoint in [('html','https://html.duckduckgo.com/html/'),('lite','https://lite.duckduckgo.com/lite/'),('noai','https://noai.duckduckgo.com/')]:
 query='Rust ownership borrowing documentation';h=dict(headers);h['Referer']=endpoint.split('/')[0]+'//'+urllib.parse.urlsplit(endpoint).netloc+'/'
 data=urllib.parse.urlencode({'q':query,'b':''}).encode()
 if name=='noai':endpoint+='?'+urllib.parse.urlencode({'q':query});data=None
 start=time.monotonic()
 try:
  with opener.open(urllib.request.Request(endpoint,data=data,headers=h),timeout=15) as r:
   body=r.read(1000000);text=body.decode(errors='replace');(out/f'ddg-{name}.html').write_bytes(body)
   report={'name':name,'status':r.status,'finalUrl':r.url,'elapsedSeconds':round(time.monotonic()-start,3),'challenge':'challenge-form' in text or 'anomaly-modal__instructions' in text,'htmlResultLinks':text.count('class="result__a"'),'liteResultLinks':text.count('class="result-link"'),'sha256':hashlib.sha256(body).hexdigest(),'headers':dict(r.headers)}
 except Exception as e:report={'name':name,'error':str(e)}
 reports.append(report);print(json.dumps({k:v for k,v in report.items() if k!='headers'}),flush=True)
(out/'ddg-variants.json').write_text(json.dumps(reports,indent=2)+'\n')
