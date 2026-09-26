"""Owned external WebKit browser, launched by GIO on an isolated display.
No URL/state/code is logged. The fixture grants its own single TLS certificate.
"""
import json,os,sys,urllib.parse
from pathlib import Path
import gi
gi.require_version('Gtk','3.0');gi.require_version('WebKit2','4.1')
from gi.repository import Gtk,WebKit2,Gio,GLib
root=Path(__file__).resolve().parent
assert '/native-candidate-mcp-c2-' in str(root)
assert len(sys.argv)==2 and len(sys.argv[1])<=131072
uri=sys.argv[1];parsed=urllib.parse.urlsplit(uri);origin=(root/'peer/origin').read_text();assert uri.startswith(origin+'/authorize?')
query=urllib.parse.parse_qs(parsed.query);redirect=query['redirect_uri'][0]
callback=urllib.parse.urlsplit(redirect);assert callback.scheme=='http' and callback.hostname in ('127.0.0.1','::1') and callback.path=='/ag1/mcp/oauth/callback'
context=WebKit2.WebContext.new_ephemeral();context.allow_tls_certificate_for_host(Gio.TlsCertificate.new_from_pem((root/'peer/cert.pem').read_text(),-1),'127.0.0.1')
window=Gtk.Window(title='Owned MCP OAuth Browser');window.set_default_size(850,600)
view=WebKit2.WebView.new_with_context(context);window.add(view)
def status(phase):
 (root/'browser-status.json').write_text(json.dumps({'phase':phase,'pid':os.getpid(),'callback_port':callback.port}))
def loaded(web,event):
 if event==WebKit2.LoadEvent.FINISHED:
  current=web.get_uri() or ''
  if current.startswith(origin+'/authorize?'):
   status('consent');web.run_javascript("document.querySelector('button').focus()",None,None,None)
  elif current.startswith(redirect+'?'):
   status('callback');GLib.timeout_add(200,lambda:(Gtk.main_quit(),False)[1])
def policy(web,decision,kind):
 if kind==WebKit2.PolicyDecisionType.NAVIGATION_ACTION:
  target=decision.get_navigation_action().get_request().get_uri()
  if not (target.startswith(origin+'/authorize?') or target==origin+'/approve' or target.startswith(redirect+'?')):decision.ignore();return True
 return False
view.connect('load-changed',loaded);view.connect('decide-policy',policy);window.connect('destroy',Gtk.main_quit)
def control():
 if (root/'close-browser').exists():Gtk.main_quit();return False
 return True
GLib.timeout_add(50,control);window.show_all();view.grab_focus();view.load_uri(uri);status('starting');Gtk.main();status('closed')
