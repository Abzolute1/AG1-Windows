// Actual GTK/GIO/external-WebKit OAuth with an owned TLS authorization/MCP peer.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import net from 'node:net';
import crypto from 'node:crypto';
import {spawn,execFileSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
import {runNativeFixture} from './p14_native_startup_smoke.mjs';
const root=fileURLToPath(new URL('../../',import.meta.url));
let peer, origin, peerRoot;
const pause=ms=>new Promise(r=>setTimeout(r,ms));
async function stopPeer(){if(peer&&peer.exitCode===null&&peer.signalCode===null){peer.kill('SIGTERM');await new Promise(r=>peer.once('exit',r));}}
await runNativeFixture(process.argv[2],{
 prefix:'oa-',
 async beforeStart({work,privateXdg}) {
  peerRoot=path.join(work,'peer');fs.mkdirSync(peerRoot,{mode:0o700});
  const key=path.join(peerRoot,'key.pem'),cert=path.join(peerRoot,'cert.pem');
  execFileSync('/usr/bin/openssl',['req','-x509','-newkey','rsa:2048','-nodes','-days','1','-subj','/CN=owned-mcp','-addext','subjectAltName=IP:127.0.0.1','-keyout',key,'-out',cert],{stdio:'ignore'});
  fs.copyFileSync(path.join(root,'tools/probes/mcp_oauth_peer.py'),path.join(peerRoot,'peer.py'));
  fs.copyFileSync(path.join(root,'tools/probes/mcp_oauth_browser.py'),path.join(work,'browser.py'));
  const log=fs.openSync(path.join(peerRoot,'stderr.log'),'wx',0o600);
  peer=spawn('/usr/bin/python3',['-I',path.join(peerRoot,'peer.py'),peerRoot],{stdio:['ignore',log,log]});fs.closeSync(log);
  try {
   const deadline=Date.now()+10000;while(!fs.existsSync(path.join(peerRoot,'origin'))){assert.equal(peer.exitCode,null);assert.ok(Date.now()<deadline);await pause(25);}
   origin=fs.readFileSync(path.join(peerRoot,'origin'),'utf8');
   fs.writeFileSync(path.join(work,'owned-mcp-tls.json'),JSON.stringify({origin,certificate_pem:fs.readFileSync(cert,'utf8')}),{mode:0o600});
   fs.writeFileSync(path.join(work,'private-input'),'owned-client',{mode:0o600});
   const apps=path.join(privateXdg.XDG_DATA_HOME,'applications');fs.mkdirSync(apps);
   fs.writeFileSync(path.join(apps,'owned-mcp-browser.desktop'),`[Desktop Entry]\nType=Application\nName=Owned MCP OAuth Browser\nExec=/usr/bin/python3 ${path.join(work,'browser.py')} %u\nTerminal=false\nNoDisplay=true\nDBusActivatable=false\nMimeType=x-scheme-handler/http;x-scheme-handler/https;\n`);
   fs.writeFileSync(path.join(privateXdg.XDG_CONFIG_HOME,'mimeapps.list'),'[Default Applications]\nx-scheme-handler/http=owned-mcp-browser.desktop;\nx-scheme-handler/https=owned-mcp-browser.desktop;\n');
   return {env:{GTK_USE_PORTAL:'0',GIO_USE_PORTALS:'0',DBUS_SESSION_BUS_ADDRESS:`unix:path=${work}/no-session-bus`},cleanup:stopPeer};
  } catch(e){await stopPeer();throw e;}
 },
 async exercise({work,probe,wait,requests}) {
  const js=script=>probe({script});
  const input=(action,target,...args)=>JSON.parse(execFileSync('/usr/bin/python3',[path.join(root,'tools/probes/mcp_oauth_input.py'),work,action,target,...args],{encoding:'utf8',timeout:25000}));
  const click=label=>wait(()=>js(`(()=>{const e=[...document.querySelectorAll('button')].find(e=>e.getAttribute('aria-label')===${JSON.stringify(label)}||e.textContent.trim()===${JSON.stringify(label)});if(!e||e.disabled)return false;e.click();return true})()`),`click ${label}`);
  const text=value=>wait(()=>js(`document.body.innerText.includes(${JSON.stringify(value)})`),value);
  const enabled=label=>wait(()=>js(`(()=>{const e=[...document.querySelectorAll('button')].find(e=>e.textContent.trim()===${JSON.stringify(label)});return !!e&&!e.disabled})()`),`${label} available`);
  const fill=async(label,value)=>assert.equal(await js(`(()=>{const l=[...document.querySelectorAll('.mcp-form label')].find(e=>e.textContent.startsWith(${JSON.stringify(label)}));const e=l?.querySelector('input');if(!e)return false;Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(e,${JSON.stringify(value)});e.dispatchEvent(new Event('input',{bubbles:true}));return true})()`),true);
  const events=()=>fs.existsSync(path.join(peerRoot,'events.jsonl'))?fs.readFileSync(path.join(peerRoot,'events.jsonl'),'utf8').trim().split('\n').filter(Boolean).map(JSON.parse):[];
  const count=route=>events().filter(e=>e.path===route).length;
  const mode=value=>fs.writeFileSync(path.join(peerRoot,'mode'),value);
  const browser=()=>{try{return JSON.parse(fs.readFileSync(path.join(work,'browser-status.json'),'utf8'));}catch{return {};}};
  async function signin(label) {
   const before=count('/.well-known/oauth-authorization-server/issuer');
   await click(label);await wait(()=>input('status','signin').dialog,'native client registration dialog');
   input('client','signin');
   await wait(()=>count('/.well-known/oauth-authorization-server/issuer')>before,'selected issuer metadata');
   await wait(()=>input('status','signin').dialog,'native scope consent');
  }
  async function connect(){await click('Connect');await text('Review connection');await js(`document.querySelector('.mcp-review input[type=checkbox]').click();true`);await click('Connect reviewed target');await text('ready · This chat');}
  await click('Back to chat');await click('MCP');await text('No servers installed.');
  assert.equal(events().length,0);
  await click('Add server');await fill('Display name','Owned OAuth MCP');await fill('MCP URL',origin+'/mcp');await click('Save inactive');await text('No private credentials configured');assert.equal(events().length,0);
  await click('Sign in with OAuth');await wait(()=>input('status','signin').dialog,'native registration entry');
  input('screenshot','signin','oauth-registration.png');input('cancel','signin');await enabled('Sign in with OAuth');
  assert.equal(count('/authorize'),0);assert.equal(count('/token'),0);await text('No private credentials configured');
  for (const failure of ['wrong-issuer','plain-pkce','malformed-metadata']) {
   mode(failure);const before=count('/.well-known/oauth-authorization-server/issuer');
   await click('Sign in with OAuth');await wait(()=>input('status','signin').dialog,'registration before metadata failure');input('client','signin');
   await wait(()=>count('/.well-known/oauth-authorization-server/issuer')>before,'hostile metadata fetched');await enabled('Sign in with OAuth');
   await wait(()=>js(`!!document.querySelector('[role=alert]')?.textContent.trim()`),'visible metadata error');
   assert.equal(count('/authorize'),0);assert.equal(count('/token'),0);await text('No private credentials configured');
   await probe({screenshot:`oauth-${failure}.png`});
  }
  mode('normal');
  await signin('Sign in with OAuth');input('screenshot','signin','oauth-consent-wide.png');
  await probe({size:[900,600]});input('screenshot','signin','oauth-consent-narrow.png');
  input('continue','signin');await wait(()=>browser().phase==='consent','external browser consent');await wait(()=>input('status','browser').dialog,'owned browser window');
  input('screenshot','browser','oauth-external-browser.png');input('continue','browser');
  await text('OAuth credentials saved');await enabled('Sign in again');await text('Identity unavailable');assert.equal(count('/token'),1);
  assert.equal(await js(`document.querySelector('input[type=password]')===null`),true);
  assert.ok(!fs.existsSync(path.join(peerRoot,'mcp-methods.jsonl')));
  await js(`document.querySelector('[aria-label="MCP authentication"]').scrollIntoView({block:'start'});true`);await probe({screenshot:'oauth-saved-narrow.png'});
  await probe({size:[1440,900]});await click('Enable here');await connect();await click('Tools');await text('owned_read');
  await probe({screenshot:'oauth-authenticated-tools.png'});
  const methods=()=>fs.readFileSync(path.join(peerRoot,'mcp-methods.jsonl'),'utf8').trim().split('\n').filter(Boolean).map(JSON.parse);
  assert.ok(methods().length>=5&&methods().every(e=>e.authenticated));
  const oldMethods=methods().length;
  await click('Refresh credentials');await wait(()=>input('status','signin').dialog,'cancellable refresh review');input('cancel','signin');await enabled('Refresh credentials');
  assert.equal(count('/token'),1);assert.equal(methods().length,oldMethods);await text('ready · This chat');
  await click('Refresh credentials');await wait(()=>input('status','signin').dialog,'refresh review');input('screenshot','signin','oauth-refresh-review.png');input('continue','signin');
  await enabled('Refresh credentials');assert.equal(count('/token'),2);assert.equal(methods().length,oldMethods);
  await wait(()=>js(`![...document.querySelectorAll('.mcp-connection')].some(e=>e.textContent.includes('ready'))`),'refresh retires connections');
  await click('Disconnect');await connect();assert.ok(methods().length>oldMethods);
  await click('Log out locally');await text('Logged out locally');await wait(()=>js(`![...document.querySelectorAll('.mcp-connection')].some(e=>e.textContent.includes('ready'))`),'logout retires connections');await click('Disconnect');
  const afterLogout=methods().length;await click('Connect');await text('Sign in again or refresh credentials');assert.equal(methods().length,afterLogout);
  await probe({size:[900,600]});await js(`document.querySelector('[aria-label="MCP authentication"]').scrollIntoView({block:'start'});true`);await probe({screenshot:'oauth-logged-out-narrow.png'});
  await probe({size:[1440,900]});await signin('Sign in again');input('continue','signin');await wait(()=>browser().phase==='consent','reauthentication browser consent');input('continue','browser');
  await text('OAuth credentials saved');await enabled('Refresh credentials');assert.equal(count('/token'),3);await connect();
  const beforeRejected=methods().length;mode('reject-token');await click('Refresh credentials');await wait(()=>input('status','signin').dialog,'rejected refresh review');input('continue','signin');
  await enabled('Sign in again');await text('MCP OAuth client registration was rejected');
  assert.equal(count('/token'),4);assert.equal(methods().length,beforeRejected);
  assert.equal(await js(`document.querySelector('[role=alert]').textContent`),'MCP OAuth client registration was rejected');
  assert.equal(await js(`[...document.querySelectorAll('button')].some(e=>e.textContent.trim()==='Refresh credentials')`),false);
  assert.equal(await js(`[...document.querySelectorAll('.mcp-connection')].some(e=>e.textContent.includes('ready'))`),false);
  await probe({screenshot:'oauth-refresh-rejected.png'});mode('normal');await click('Disconnect');await click('Connect');await text('Sign in again or refresh credentials');assert.equal(methods().length,beforeRejected);
  await signin('Sign in again');input('continue','signin');await wait(()=>browser().phase==='consent','cancelled reauthentication browser consent');
  await wait(()=>input('status','progress').dialog,'cancellable native browser wait');const port=browser().callback_port;input('cancel','progress');await enabled('Sign in again');assert.equal(count('/token'),4);
  await wait(()=>new Promise(resolve=>{const s=net.createConnection({host:'127.0.0.1',port});s.on('connect',()=>{s.destroy();resolve(false)});s.on('error',e=>resolve(e.code==='ECONNREFUSED'));}),'cancelled callback listener closed');
  fs.writeFileSync(path.join(work,'close-browser'),'close');await wait(()=>browser().phase==='closed','owned browser exits');
  assert.equal(await js('document.documentElement.scrollWidth<=innerWidth'),true);assert.equal(requests.length,0);
  const secrets=[1,2,3].flatMap(n=>[`owned-native-oauth-access-${n}`,`owned-native-oauth-refresh-${n}`]);let scanned=0;const hits=[];
  function scan(dir){for(const ent of fs.readdirSync(dir,{withFileTypes:true})){const p=path.join(dir,ent.name);if(p===peerRoot||p.includes('/mcp-private-store'))continue;if(ent.isDirectory())scan(p);else if(ent.isFile()){const bytes=fs.readFileSync(p);scanned++;if(secrets.some(v=>bytes.includes(Buffer.from(v))))hits.push(path.relative(work,p));}}}
  scan(work);assert.deepEqual(hits,[]);fs.writeFileSync(path.join(work,'oauth-secret-scan.json'),JSON.stringify({scanned,hits,exclusions:['owned peer private files','dedicated MCP private store']},null,2));
  fs.writeFileSync(path.join(work,'oauth-events.json'),JSON.stringify(events(),null,2));
  return {providerRequests:[],promptsSubmitted:0,checks:['native GTK public client selection and scoped consent','wrong issuer, plain PKCE and malformed metadata fail before browser/token','GIO launch into an isolated external WebKit browser','actual HTTPS authorization code/S256 exchange','authenticated MCP discovery and tool inspection','refresh review cancellation sends nothing','rejected refresh displays a native error without replay or hostile error reflection','refresh rotates credentials without replay','local logout prevents fallback','native cancel closes callback listener','wide/narrow screenshots','literal access/refresh secret scan'],scannedFiles:scanned,limitations:['No live account, inference or server capability invocation','Test-only single-origin TLS policy; screenshots require inspection','Public pre-registration only']};
 }
});
