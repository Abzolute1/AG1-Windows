import urllib.request,urllib.parse,json,time,hashlib
from html.parser import HTMLParser
from pathlib import Path
out=Path('.local/free-search-recovery-20260914');opener=urllib.request.build_opener(urllib.request.ProxyHandler({}))
class Parser(HTMLParser):
 def handle_starttag(self,tag,attrs):
  a=dict(attrs)
  if tag=='script' and a.get('id')=='deep_preload_script':self.url=a.get('src')
headers={'User-Agent':'Mozilla/5.0','Accept':'text/html,application/xhtml+xml','Accept-Language':'en-US,en;q=0.9'}
reports=[]
for i,query in enumerate(['Rust ownership borrowing documentation','Norway public transport Oslo','site:docs.python.org free threading GIL']):
 try:
  landing='https://noai.duckduckgo.com/?'+urllib.parse.urlencode({'q':query})
  start=time.monotonic()
  with opener.open(urllib.request.Request(landing,headers=headers),timeout=15) as r:
   body=r.read(1000000);(out/f'noai-native-{i}-landing.html').write_bytes(body);page_status=r.status
  parser=Parser();parser.url=None;parser.feed(body.decode());url=parser.url
  if not url:raise RuntimeError('No result preload URL')
  u=urllib.parse.urlsplit(url)
  if u.scheme!='https' or u.netloc!='links.duckduckgo.com' or u.path!='/d.js':raise RuntimeError('Unexpected preload destination')
  h=dict(headers,Referer='https://noai.duckduckgo.com/');h['Accept']='*/*'
  with opener.open(urllib.request.Request(url,headers=h),timeout=15) as r:
   body=r.read(1000000);(out/f'noai-native-{i}-results.js').write_bytes(body)
   report={'query':query,'landing':landing,'landingStatus':page_status,'resultUrl':url,'status':r.status,'seconds':round(time.monotonic()-start,3),'bytes':len(body),'sha256':hashlib.sha256(body).hexdigest(),'headers':dict(r.headers),'preview':body.decode(errors='replace')[:500]}
 except Exception as e:report={'query':query,'error':str(e)}
 reports.append(report);print(json.dumps({k:v for k,v in report.items() if k not in ['headers','resultUrl']}),flush=True)
 if report.get('error'):break
(out/'noai-native.json').write_text(json.dumps(reports,indent=2)+'\n')
