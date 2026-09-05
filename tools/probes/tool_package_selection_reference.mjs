import fs from 'node:fs';
import path from 'node:path';
import assert from 'node:assert/strict';
import { fileURLToPath, pathToFileURL } from 'node:url';
await import('../offline_guard.mjs');
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const { DefaultPackageManager } = await import(pathToFileURL(path.join(root,
  'reference/ag1-executable/packages/coding-agent/dist/core/package-manager.js')));
const fixture = fs.mkdtempSync(path.join(root, '.local/p09-package-selection-'));
const prior = process.env.AG1_OFFLINE;
const cases = [
  { action: 'update', user: ['npm:one', 'npm:two@1'], project: ['npm:one', './local'] },
  { action: 'update', source: 'npm:one@2', user: ['npm:one'], project: [{ source: 'npm:one@1', skills: [] }] },
  { action: 'update', source: 'one', user: ['npm:one@1'], project: ['npm:one'] },
  { action: 'update', source: 'github.com/acme/repo', user: ['git:github.com/acme/repo'], project: [] },
  { action: 'update', source: 'github.com/acme/repo@main', user: ['git:github.com/acme/repo@main'], project: [] },
  { action: 'update', source: 'npm:absent', user: ['npm:one'], project: [] },
  { action: 'update', source: './local', user: ['./local'], project: ['./local', '../local'] },
  { action: 'update', source: '', user: ['npm:one'], project: [] },
  { action: 'checks', user: ['npm:one', 'npm:two', 'git:github.com/acme/repo'], project: ['npm:one@1', 'npm:three', 'npm:three@2'] },
  { action: 'checks', user: ['git:github.com/acme/repo', 'npm:two'], project: ['https://github.com/acme/repo', './local', 'npm:two'] },
  { action: 'checks', user: ['npm:one@1', 'git:github.com/acme/repo@main'], project: [] },
  { action: 'checks', offline: true, user: ['npm:one'], project: ['npm:two'] },
];
const rows = [];
try {
  for (const [index, input] of cases.entries()) {
    const dir = path.join(fixture, String(index));
    const cwd = path.join(dir, 'project'), agentDir = path.join(dir, 'agent');
    for (const p of [cwd, agentDir]) fs.mkdirSync(p, { recursive: true });
    const pm = new DefaultPackageManager({ cwd, agentDir, settingsManager: {
      getGlobalSettings: () => ({ packages: input.user }),
      getProjectSettings: () => ({ packages: input.project }),
      getNpmCommand: () => undefined,
    } });
    const calls = [];
    pm.getGlobalNpmRoot = () => path.join(dir, 'npm-global/node_modules');
    pm.updateSourceForScope = async (source, scope) => { calls.push({ source, scope }); };
    // Query selection is isolated from transport: every installed unpinned
    // candidate reports an update; actual query methods are tested separately.
    pm.npmHasAvailableUpdate = async () => true;
    pm.gitHasAvailableUpdate = async () => true;
    for (const scope of ['user', 'project']) for (const entry of input[scope]) {
      const source = typeof entry === 'string' ? entry : entry.source;
      const parsed = pm.parseSource(source);
      if (parsed.type === 'local') continue;
      const installed = parsed.type === 'npm' ? pm.getNpmInstallPath(parsed, scope) : pm.getGitInstallPath(parsed, scope);
      assert(installed.startsWith(dir + path.sep));
      fs.mkdirSync(installed, { recursive: true });
    }
    process.env.AG1_OFFLINE = input.offline ? '1' : '0';
    let output;
    try {
      if (input.action === 'update') { await pm.update(input.source); output = { value: calls }; }
      else output = { value: await pm.checkForAvailableUpdates() };
    } catch (error) { output = { error: error.message }; }
    rows.push({ input, output });
  }
} finally {
  if (prior === undefined) delete process.env.AG1_OFFLINE; else process.env.AG1_OFFLINE = prior;
  fs.rmSync(fixture, { recursive: true, force: true });
}
const result = { version: 1, source_commit: 'e30f0b4b238ee5618928bcd17b4571eb046e7038',
  port: 'Owned installed directories; update execution and remote-query outcomes replaced only on each SDK fixture instance', rows };
const target = path.join(root, 'fixtures/tools/package-selection-oracle.json');
if (process.argv.includes('--capture')) { assert(!fs.existsSync(target)); fs.writeFileSync(target, JSON.stringify(result, null, 2) + '\n'); }
else assert.deepEqual(result, JSON.parse(fs.readFileSync(target, 'utf8')));
console.log(JSON.stringify({ status: 'passed', package_selection_cases: rows.length, external_commands: 0, network_actions: 0 }));
