// CLI: bounded native WebKit startup, no prompt. Exported runner also supports
// explicitly supplied controlled workflow fixtures on a read-only root.
// Requires an explicitly supplied visual-test candidate; never uses /opt/ag1.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import net from 'node:net';
import http from 'node:http';
import crypto from 'node:crypto';
import os from 'node:os';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
const root = fileURLToPath(new URL('../../', import.meta.url));
export async function runNativeFixture(candidatePath, options = {}) {
const candidate = fs.realpathSync(candidatePath);
assert.ok(candidate.startsWith(path.join(root, '.local/p14-live-fixes/native-candidate-')));
const binary = path.join(candidate, 'ag1-desktop');
assert.ok(fs.statSync(binary).isFile());
const work = fs.mkdtempSync(path.join(candidate, options.prefix || 'startup-'));
const socket = path.join(work, 'control.sock');
assert.ok(Buffer.byteLength(socket) < 108);
const requests = [];
const sink = http.createServer((request, response) => {
  requests.push({ method: request.method, url: request.url });
  if (options.handleRequest) return options.handleRequest(request, response);
  response.writeHead(503, { 'Content-Type': 'application/json' });
  response.end('{"error":{"message":"No inference is permitted in this startup smoke"}}');
});
await new Promise(resolve => sink.listen(0, '127.0.0.1', resolve));
const config = { models: [{ provider: 'fixture', id: 'startup-fixture', api: 'openai-completions',
  baseUrl: `http://127.0.0.1:${sink.address().port}`, contextWindow: 32768, maxTokens: 1024,
  input: ['text'], reasoning: false }], selected: 'startup-fixture', credential_env: null,
  system_prompt: options.exercise ? 'Controlled native approval fixture. No external services.' : 'Startup-only fixture; no messages may be submitted.',
  max_generations: 3, fixture: true, policy_mode: 'interactive' };
if (options.gitReview) {
  const helper = path.join(candidate, 'ag1-process-host');
  assert.ok(fs.statSync(helper).isFile(), 'Review fixture requires an isolated process helper');
  config.tooling = { process_host: helper, git: '/usr/bin/git', rg: '/usr/bin/true',
    fd: '/usr/bin/fd', bubblewrap: '/usr/bin/bwrap', targets: {
      node: null, npm: null, cargo: null, rustc: null, godot: null, mounts: [] } };
}
if (options.ownedRuntime) {
  assert.ok(options.exercise && !options.handleRequest, 'Owned runtime supplies its own fixed-response server');
  // Reserve an ephemeral endpoint, then let the desktop own its server process.
  // A bind collision fails the fixture; it never attaches to or kills another server.
  await new Promise(resolve => sink.close(resolve));
}
const configuration = path.join(work, 'host.json');
fs.writeFileSync(configuration, JSON.stringify(config), { mode: 0o600 });
const display = Array.from({ length: 200 }, (_, i) => i + 300).find(n =>
  !fs.existsSync(`/tmp/.X${n}-lock`) && !fs.existsSync(`/tmp/.X11-unix/X${n}`));
assert.ok(display);
const auth = path.join(work, 'display.auth');
const field = data => { const length = Buffer.alloc(2); length.writeUInt16BE(data.length); return Buffer.concat([length, data]); };
fs.writeFileSync(auth, Buffer.concat([Buffer.from([255, 255]), ...[Buffer.alloc(0), Buffer.from(String(display)),
  Buffer.from('MIT-MAGIC-COOKIE-1'), crypto.randomBytes(16)].map(field)]), { mode: 0o600 });
function start(command, args, env, name) {
  const log = fs.openSync(path.join(work, name + '.log'), 'wx', 0o600);
  const child = spawn(command, args, { env, cwd: root, detached: true, stdio: ['ignore', log, log] });
  fs.closeSync(log);
  child.on('error', error => { child.startError = error; });
  return child;
}
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
async function waitUntil(predicate, child, description) {
  const deadline = Date.now() + 20000;
  while (!await predicate()) {
    if (child.startError) throw child.startError;
    assert.equal(child.exitCode, null, `${description}: process exited`);
    assert.equal(child.signalCode, null, `${description}: process signalled`);
    assert.ok(Date.now() < deadline, `${description}: timeout; inspect ${work}`);
    await delay(100);
  }
}
function probe(request) {
  return new Promise((resolve, reject) => {
    const connection = net.createConnection(socket);
    let data = '';
    connection.setTimeout(16000, () => connection.destroy(new Error('Visual probe timeout')));
    connection.on('connect', () => connection.write(JSON.stringify(request) + '\n'));
    connection.on('error', reject);
    connection.on('data', bytes => {
      data += bytes;
      if (data.length > 1048576) connection.destroy(new Error('Visual response too large'));
      else if (data.includes('\n')) {
        connection.end();
        try { const result = JSON.parse(data.split('\n')[0]);
          if ('error' in result) reject(new Error(result.error)); else resolve(result.ok);
        } catch (error) { reject(error); }
      }
    });
  });
}
async function stop(child) {
  if (!child?.pid) return;
  try { process.kill(-child.pid, 'SIGTERM'); } catch (e) { if (e.code !== 'ESRCH') throw e; }
  const deadline = Date.now() + 5000;
  while (child.exitCode === null && child.signalCode === null && Date.now() < deadline) await delay(50);
  // Drain any remaining descendants in this owned process group as well.
  try { process.kill(-child.pid, 'SIGKILL'); } catch (e) { if (e.code !== 'ESRCH') throw e; }
}
const cleanEnv = Object.fromEntries(Object.entries(process.env).filter(([name]) => !name.startsWith('AG1_')));
// AG1_DESKTOP_STATE does not isolate WebKit's localStorage/cache. Keep those
// under the owned fixture too, without touching the user's saved preferences.
const privateXdg = {};
for (const kind of ['DATA', 'CONFIG', 'CACHE']) {
  const directory = path.join(work, `xdg-${kind.toLowerCase()}`);
  fs.mkdirSync(directory, { mode: 0o700 });
  privateXdg[`XDG_${kind}_HOME`] = directory;
}
let xvfb, desktop, prepared;
try {
  xvfb = start(path.join(root, '.local/p11/xvfb/usr/bin/Xvfb'), [`:${display}`, '-screen', '0', '1440x900x24',
    '-nolisten', 'tcp', '-auth', auth], cleanEnv, 'xvfb');
  await waitUntil(() => fs.existsSync(`/tmp/.X11-unix/X${display}`), xvfb, 'private display');
  if (options.beforeStart) prepared = await options.beforeStart({ work, privateXdg, display, auth });
  // The native authority registry intentionally ignores XDG overrides. Shadow
  // it in a private mount namespace for workflow fixtures; never register the
  // disposable project in the user's real authority namespace.
  let command = binary, args = [];
  if (options.exercise) {
    const registry = path.join(work, 'authority');
    fs.mkdirSync(registry, { mode: 0o700 });
    command = '/usr/bin/bwrap';
    // /proc must describe the private PID namespace: native helpers resolve
    // pinned descriptor paths using the desktop's namespace-local process ID.
    args = ['--die-with-parent', '--unshare-pid', '--ro-bind', '/', '/', '--proc', '/proc', '--dev-bind', '/dev', '/dev',
      '--bind', work, work, '--bind', registry, path.join(os.userInfo().homedir, '.local/state/ag1/authority-v1'),
      '--tmpfs', '/tmp', '--ro-bind', '/tmp/.X11-unix', '/tmp/.X11-unix', '--', binary];
  }
  desktop = start(command, args, { ...cleanEnv, ...privateXdg, DISPLAY: `:${display}`, XAUTHORITY: auth,
    ...(prepared?.env || {}),
    GDK_BACKEND: 'x11', LIBGL_ALWAYS_SOFTWARE: '1', WEBKIT_DISABLE_DMABUF_RENDERER: '1',
    AG1_NATIVE_CONFIG: configuration, AG1_FIXTURE_PROVIDER: '1', AG1_INSTALL_ROOT: root,
    ...(options.ownedRuntime ? { AG1_P14_OWNED_FIXTURE: '1' } : {}),
    ...(options.githubFixture ? { AG1_VISUAL_GITHUB_FIXTURE: 'owned-offline-v1' } : {}),
    AG1_DESKTOP_STATE: path.join(work, 'state'), AG1_VISUAL_SOCKET: socket }, 'desktop');
  await waitUntil(() => fs.existsSync(socket), desktop, 'native visual broker');
  await waitUntil(() => probe({ script: 'document.body.innerText.includes("What’s on your mind?")' }), desktop, 'native home');
  await waitUntil(() => probe({ script: 'document.body.innerText.includes("startup-fixture")' }), desktop, 'model metadata bootstrap');
  const view = await probe({ script: '({width:innerWidth,height:innerHeight,dpr:devicePixelRatio,scripts:[...document.scripts].map(s=>s.src),text:document.body.innerText})' });
  assert.equal(view.width, 1440); assert.equal(view.height, 900);
  const index = fs.readFileSync(path.join(candidate, 'renderer/index.html'), 'utf8');
  const entry = index.match(/src="([^"]+\.js)"/)[1];
  assert.ok(view.scripts.some(src => src.endsWith(entry)), JSON.stringify(view.scripts));
  assert.match(view.text, /Ask before changes/);
  assert.match(view.text, /startup-fixture/);
  await probe({ screenshot: 'native-home.png' });
  assert.equal(await probe({ script: '(()=>{const button=[...document.querySelectorAll("button")].find(b=>b.getAttribute("aria-label")==="Settings");if(!button)return false;button.click();return true})()' }), true);
  await waitUntil(() => probe({ script: 'document.body.innerText.includes("Appearance")' }), desktop, 'native settings');
  await probe({ screenshot: 'native-settings.png' });
  let workflow;
  if (options.exercise) workflow = await options.exercise({ work, probe,
    wait: (predicate, description) => waitUntil(predicate, desktop, description), requests });
  else assert.deepEqual(requests, []);
  const result = { status: 'passed', binary, sha256: crypto.createHash('sha256').update(fs.readFileSync(binary)).digest('hex'),
    embeddedEntry: entry, viewport: { width: view.width, height: view.height, dpr: view.dpr },
    nativeEngine: 'Linux Tauri/WebKit', driver: 'visual-test broker', display: 'private authenticated Xvfb',
    checks: ['current embedded renderer entry', 'home visible', 'safe approval default', 'fixture model selected', 'settings visible'],
    providerRequests: workflow?.providerRequests ?? requests, promptsSubmitted: workflow?.promptsSubmitted ?? 0,
    workflow, liveInference: false, installedRelease: false, work };
  fs.writeFileSync(path.join(work, 'result.json'), JSON.stringify(result, null, 2) + '\n');
  console.log(JSON.stringify(result, null, 2));
} catch (error) {
  fs.writeFileSync(path.join(work, 'failure.json'), JSON.stringify({ status: 'failed',
    error: String(error), providerRequests: requests, work }, null, 2) + '\n');
  try { fs.writeFileSync(path.join(work, 'failure-view.json'), JSON.stringify(await probe({
    script: '({text:document.body.innerText,alerts:[...document.querySelectorAll("[role=alert]")].map(e=>e.textContent)})'
  }), null, 2)); } catch { /* Preserve the original failure even if the UI exited. */ }
  throw error;
} finally {
  try { await stop(desktop); } finally {
    try { await prepared?.cleanup?.(); } finally {
    try { await stop(xvfb); } finally {
      sink.closeAllConnections(); if (sink.listening) await new Promise(resolve => sink.close(resolve));
    }
    }
  }
}
}
if (process.argv[1] && fs.realpathSync(process.argv[1]) === fileURLToPath(import.meta.url)) {
  await runNativeFixture(process.argv[2]);
}
