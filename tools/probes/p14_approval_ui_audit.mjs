// Real Chromium, current renderer source, in-memory bridge only. No native
// backend, model endpoint, installed app or user's desktop is contacted.
import assert from 'node:assert/strict';
import { mkdtemp, writeFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import { createServer } from '../../product/apps/desktop/node_modules/vite/dist/node/index.js';

const root = fileURLToPath(new URL('../../', import.meta.url));
const { chromium } = await import(process.env.AG1_PLAYWRIGHT_MODULE || 'playwright');
const output = await mkdtemp(path.join(root, '.local/p14-live-fixes/approval-browser-'));
const server = await createServer({
  root: path.join(root, 'product/apps/desktop'),
  server: { host: '127.0.0.1', port: 0, strictPort: false, open: false },
});
let browser;
try {
  await server.listen();
  const origin = server.resolvedUrls.local[0];
  browser = await chromium.launch({ headless: true });
  const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
  page.setDefaultTimeout(15000);
  const errors = [], blocked = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.route('**/*', route => {
    const url = new URL(route.request().url());
    if (url.origin !== new URL(origin).origin && !['data:', 'blob:'].includes(url.protocol)) {
      blocked.push(url.href);
      return route.abort();
    }
    return route.continue();
  });
  await page.addInitScript(() => {
    const scope = { workspace: 'w', session: 's', instance: 'i', epoch: '1' };
    const snapshot = { version: { major: 1, minor: 0, schema: 'fixture' }, scope,
      revision: '1', watermark: '1', workspace_path: '/fixture', status: 'awaiting_input',
      active_operation: 'operation', messages: [], message_count: 0, pending: [],
      model: 'fixture', diagnostic: null, session_file: null, fixture_provider: true };
    let serial = 0;
    window.__audit = { commands: [], unexpected: [], replace: (lifetime = 60000) => {
      serial++;
      const binding = { id: `request-${serial}`, operation: `operation-${serial}`, argument_sha256: 'fixture-args',
        scope, revision: String(serial), expires_ms: String(Date.now() + lifetime) };
      snapshot.revision = String(serial);
      snapshot.pending = [{ binding, kind: 'approval', options: [], evidence: 'fixture-request', truncated: false,
        title: JSON.stringify({ summary: 'Write notes.txt', details: 'Fixture project only',
          changes: 'Path: notes.txt\n\nProposed content:\nA reviewed change <script>literal text</script>' }) }];
      return binding;
    } };
    window.__audit.replace(8000);
    window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
    window.__TAURI_INTERNALS__ = { transformCallback: () => 1, unregisterCallback: () => {}, invoke: async (name, args) => {
      if (name === 'catalog') return { projects: ['/fixture'], theme: 'dark', chats: [] };
      if (name === 'desktop_preferences') return { approval_mode: 'interactive' };
      if (name === 'desktop_models') return [];
      if (name === 'bridge_view') return { snapshot: structuredClone(snapshot), error: null, progress: '' };
      if (name.startsWith('plugin:event|')) return 1;
      if (name === 'bridge_command') {
        const command = args.envelope.command;
        window.__audit.commands.push(command);
        const pending = snapshot.pending[0];
        if (command.kind !== 'approve' || !pending || JSON.stringify(command.binding) !== JSON.stringify(pending.binding)) {
          window.__audit.unexpected.push(command);
          throw new Error('Unexpected fixture command');
        }
        if (command.allow && Date.now() >= Number(pending.binding.expires_ms)) {
          window.__audit.unexpected.push(command);
          throw new Error('Expired approval reached fixture bridge');
        }
        snapshot.pending = [];
        return { kind: 'reply', id: args.envelope.id, scope, revision: snapshot.revision,
          durable: false, status: 'accepted', data: null };
      }
      window.__audit.unexpected.push(name);
      throw new Error('Unexpected bridge call ' + name);
    } };
  });
  await page.goto(origin);
  const dialog = page.getByRole('dialog', { name: 'Review tool request' });
  const allow = page.getByRole('button', { name: 'Allow this operation', exact: true });
  await dialog.waitFor();
  assert.equal(await allow.isEnabled(), true);
  assert.match(await dialog.innerText(), /<script>literal text<\/script>/);
  assert.equal(await dialog.locator('script').count(), 0);
  await allow.focus();
  await page.keyboard.press('Enter');
  assert.deepEqual(await page.evaluate(() => window.__audit.commands), []);
  await page.screenshot({ path: path.join(output, 'review-before-expiry.png') });
  await page.getByText(/This request expired/).waitFor();
  assert.equal(await allow.isDisabled(), true);
  assert.deepEqual(await page.evaluate(() => window.__audit.commands), []);
  await page.screenshot({ path: path.join(output, 'review-expired.png') });

  const binding = await page.evaluate(() => window.__audit.replace());
  await page.getByText(/Review expires in/).waitFor();
  assert.equal(await allow.isEnabled(), true);
  const dimensions = [];
  for (const viewport of [{ width: 1440, height: 900 }, { width: 900, height: 600 }]) {
    await page.setViewportSize(viewport);
    const box = await dialog.boundingBox();
    assert.ok(box && box.x >= 0 && box.y >= 0 && box.x + box.width <= viewport.width + 1 && box.y + box.height <= viewport.height + 1, JSON.stringify(box));
    assert.equal(await allow.isVisible(), true);
    dimensions.push({ viewport, dialog: box });
  }
  await page.screenshot({ path: path.join(output, 'review-small.png') });
  await allow.click();
  await dialog.waitFor({ state: 'hidden' });
  assert.deepEqual(await page.evaluate(() => window.__audit.commands), [{ kind: 'approve', binding, allow: true }]);
  const denied = await page.evaluate(() => window.__audit.replace());
  await dialog.waitFor();
  await page.getByRole('button', { name: 'Close Review tool request', exact: true }).click();
  await dialog.waitFor({ state: 'hidden' });
  assert.deepEqual(await page.evaluate(() => window.__audit.commands), [
    { kind: 'approve', binding, allow: true }, { kind: 'approve', binding: denied, allow: false },
  ]);
  assert.deepEqual(await page.evaluate(() => window.__audit.unexpected), []);
  assert.deepEqual(blocked, []);
  assert.deepEqual(errors, []);
  const result = { status: 'passed', nativeBridge: false, inference: false, installedApp: false,
    renderer: 'current source via owned loopback Vite server', engine: 'headless Chromium',
    checks: ['literal preview', 'Enter blocked', 'unchanged snapshot expiry disables Allow',
      'fresh replacement enables Allow', 'exact binding approved', 'close denies', 'dialog fits both viewports'],
    dimensions, output };
  await writeFile(path.join(output, 'result.json'), JSON.stringify(result, null, 2) + '\n');
  console.log(JSON.stringify(result, null, 2));
} finally {
  try { if (browser) await browser.close(); }
  finally { await server.close(); }
}
