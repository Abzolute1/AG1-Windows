"""Owned HTTPS OAuth/MCP peer. Logs route names/counts only, never private fields."""
import base64,hashlib,html,http.server,json,secrets,ssl,sys,threading,time,urllib.parse
from pathlib import Path
root=Path(sys.argv[1]).resolve();assert '/native-candidate-mcp-c2-' in str(root)
lock=threading.Lock();attempts={};codes={};refreshes={};accesses=set();generation=0
def mode():return (root/'mode').read_text() if (root/'mode').exists() else 'normal'
class Peer(http.server.BaseHTTPRequestHandler):
 def log_message(self,*args):pass
 def event(self,path,authenticated=False):
  with lock:
   with (root/'events.jsonl').open('a') as f:f.write(json.dumps({'method':self.command,'path':path,'authenticated':authenticated})+'\n')
 def reply(self,status,body=b'',kind='application/json',headers=()):
  self.send_response(status);self.send_header('Content-Type',kind);self.send_header('Content-Length',str(len(body)));self.send_header('Cache-Control','no-store');self.send_header('Connection','close')
  for k,v in headers:self.send_header(k,v)
  self.end_headers()
  try:self.wfile.write(body)
  except (BrokenPipeError,ConnectionResetError,ssl.SSLError):pass
 def json(self,value,status=200):self.reply(status,json.dumps(value).encode())
 def do_GET(self):
  u=urllib.parse.urlsplit(self.path);p=u.path;self.event(p)
  if p=='/mcp':return self.reply(401,headers=[('WWW-Authenticate',f'Bearer resource_metadata="{origin}/resource", scope="files:read"')])
  if p=='/resource':return self.json({'resource':origin+'/mcp','authorization_servers':[origin+'/issuer'],'scopes_supported':['files:read']})
  if p=='/.well-known/oauth-authorization-server/issuer':
   if mode()=='malformed-metadata':return self.reply(200,b'{')
   return self.json({'issuer':origin+('/foreign' if mode()=='wrong-issuer' else '/issuer'),'authorization_endpoint':origin+'/authorize','token_endpoint':origin+'/token','response_types_supported':['code'],'code_challenge_methods_supported':['plain' if mode()=='plain-pkce' else 'S256'],'token_endpoint_auth_methods_supported':['none'],'authorization_response_iss_parameter_supported':True})
  if p=='/authorize':
   values=urllib.parse.parse_qs(u.query,strict_parsing=True)
   assert all(len(v)==1 for v in values.values());q={k:v[0] for k,v in values.items()}
   assert q['client_id']=='owned-client' and q['resource']==origin+'/mcp' and q['code_challenge_method']=='S256' and q['scope']=='files:read'
   redirect=urllib.parse.urlsplit(q['redirect_uri']);assert redirect.scheme=='http' and redirect.hostname in ('127.0.0.1','::1') and redirect.path=='/ag1/mcp/oauth/callback'
   key=secrets.token_urlsafe(24)
   with lock:attempts[key]=q
   body=f'''<!doctype html><meta charset="utf-8"><title>Owned MCP authorization</title><body style="font:20px sans-serif;margin:50px;background:#fafafa;color:#202020"><h1>Owned MCP authorization</h1><p>This is an isolated fixture account.</p><p>AG1 requests: <strong>files:read</strong></p><form method="post" action="/approve"><input type="hidden" name="attempt" value="{html.escape(key)}"><button autofocus style="font:inherit;padding:16px">Approve fixture access</button></form></body>'''.encode()
   return self.reply(200,body,'text/html; charset=utf-8')
  self.reply(404)
 def do_POST(self):
  global generation
  p=urllib.parse.urlsplit(self.path).path;n=int(self.headers.get('Content-Length','0'));assert 0<n<=262144
  raw=self.rfile.read(n)
  if p in ('/approve','/token'):
   parsed=urllib.parse.parse_qs(raw.decode(),strict_parsing=True);assert all(len(v)==1 for v in parsed.values());q={k:v[0] for k,v in parsed.items()};self.event(p)
   if p=='/approve':
    with lock:auth=attempts.pop(q['attempt']);code=secrets.token_urlsafe(32);codes[code]=auth
    query=urllib.parse.urlencode({'code':code,'state':auth['state'],'iss':origin+'/issuer'})
    return self.reply(303,headers=[('Location',auth['redirect_uri']+'?'+query)])
   assert q['client_id']=='owned-client' and q['resource']==origin+'/mcp'
   if mode()=='reject-token':return self.json({'error':'invalid_client','error_description':'owned hostile error text must not be forwarded'},400)
   with lock:
    if q['grant_type']=='authorization_code':
     auth=codes.pop(q['code']);challenge=base64.urlsafe_b64encode(hashlib.sha256(q['code_verifier'].encode()).digest()).rstrip(b'=').decode();assert challenge==auth['code_challenge'] and q['redirect_uri']==auth['redirect_uri']
    else:assert q['grant_type']=='refresh_token' and refreshes.pop(q['refresh_token']) is True
    generation+=1;access='owned-native-oauth-access-'+str(generation);refresh='owned-native-oauth-refresh-'+str(generation);accesses.add(access);refreshes[refresh]=True
   return self.json({'access_token':access,'refresh_token':refresh,'token_type':'Bearer','expires_in':3600,'scope':'files:read'})
  if p=='/mcp':
   token=self.headers.get('Authorization','').removeprefix('Bearer ')
   with lock:authenticated=token in accesses
   self.event(p,authenticated)
   if not authenticated:return self.reply(401)
   request=json.loads(raw);method=request['method'];params=request.get('params',{})
   with (root/'mcp-methods.jsonl').open('a') as f:f.write(json.dumps({'method':method,'authenticated':True})+'\n')
   if method=='server/discover':result={'resultType':'complete','supportedVersions':['2026-07-28'],'capabilities':{'tools':{},'resources':{},'prompts':{}},'ttlMs':0,'cacheScope':'private'}
   elif method=='tools/list':result={'tools':[{'name':'owned_read','description':'Owned OAuth read fixture','inputSchema':{'type':'object','properties':{},'additionalProperties':False}}]}
   elif method=='resources/list':result={'resources':[]}
   elif method=='resources/templates/list':result={'resourceTemplates':[]}
   elif method=='prompts/list':result={'prompts':[]}
   else:return self.json({'jsonrpc':'2.0','id':request.get('id'),'error':{'code':-32601,'message':'Unsupported fixture method'}})
   return self.json({'jsonrpc':'2.0','id':request['id'],'result':result})
  self.reply(404)
server=http.server.ThreadingHTTPServer(('127.0.0.1',0),Peer);server.daemon_threads=True
origin='https://127.0.0.1:'+str(server.server_port)
context=ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER);context.load_cert_chain(root/'cert.pem',root/'key.pem');server.socket=context.wrap_socket(server.socket,server_side=True)
(root/'origin').write_text(origin)
server.serve_forever(poll_interval=.05)
