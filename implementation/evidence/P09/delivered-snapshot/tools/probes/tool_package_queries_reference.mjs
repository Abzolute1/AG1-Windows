import fs from 'node:fs';
import path from 'node:path';
import assert from 'node:assert/strict';
import { fileURLToPath, pathToFileURL } from 'node:url';
await import('../offline_guard.mjs');
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const { DefaultPackageManager } = await import(pathToFileURL(path.join(root, 'reference/ag1-executable/packages/coding-agent/dist/core/package-manager.js')));
const fixture = fs.mkdtempSync(path.join(root, '.local/p09-package-queries-'));
const prior = process.env.AG1_OFFLINE, priorFetch = globalThis.fetch;
const cases = [
  ...[
    { installed: '1', latest: '2' }, { installed: '1', latest: '1' },
    { installed: '1', latest: '0' }, { installed: '', latest: '2' },
    { latest: '2' }, { installed: '1' }, { installed: '1', latest: null },
    { installed: '1', latest: 1 }, { installed: 1, latest: 1 },
    { installed: {}, latest: {} }, { installed: '1', fail: true },
    { installed: '1', badStatus: true },
  ].map(input => ({ kind: 'npm', ...input })),
  ...[
    {}, { same: true }, { noUpstream: true }, { upstream: 'elsewhere/main' },
    { upstream: 'origin/' }, { upstream: 'origin/topic/branch' },
    { emptyUpstreamHead: true }, { badHead: true, noUpstream: true },
    { failRemote: true }, { failLocal: true },
  ].map(input => ({ kind: 'git', ...input })),
];
const rows = [];
try {
  process.env.AG1_OFFLINE = '0';
  for (const [index, input] of cases.entries()) {
    const dir = path.join(fixture, String(index)); fs.mkdirSync(dir);
    const pm = new DefaultPackageManager({ cwd: dir, agentDir: dir, settingsManager: {} });
    const calls = [];
    globalThis.fetch = async (url, options) => {
      calls.push({ kind: 'registry', url, signal: options.signal instanceof AbortSignal });
      if (input.fail) throw new Error('fixture offline');
      return { ok: !input.badStatus, status: input.badStatus ? 503 : 200,
        json: async () => Object.hasOwn(input, 'latest') ? { version: input.latest } : {} };
    };
    pm.runCommandCapture = async (command, args, options) => {
      calls.push({ kind: 'git', command, args, options: { ...options, cwd: '/fixture/installed' } });
      if (args.join(' ') === 'rev-parse HEAD') { if (input.failLocal) throw new Error('fixture failed'); return 'a'.repeat(40) + '\n'; }
      if (args[0] === 'rev-parse') { if (input.noUpstream) throw new Error('no upstream'); return input.upstream ?? 'origin/main\n'; }
      if (input.failRemote) throw new Error('fixture remote failed');
      if (args[2] !== 'HEAD' && input.emptyUpstreamHead) return '';
      if (input.badHead) return 'invalid\tHEAD';
      return (input.same ? 'a' : 'b').repeat(40) + '\t' + args[2] + '\n';
    };
    let value;
    if (input.kind === 'npm') {
      fs.writeFileSync(path.join(dir, 'package.json'), JSON.stringify(Object.hasOwn(input, 'installed') ? { version: input.installed } : {}));
      value = await pm.npmHasAvailableUpdate(pm.parseSource('npm:@scope/fixture'), dir);
    } else value = await pm.gitHasAvailableUpdate(dir);
    rows.push({ input, value, calls });
  }
} finally {
  globalThis.fetch = priorFetch;
  if (prior === undefined) delete process.env.AG1_OFFLINE; else process.env.AG1_OFFLINE = prior;
  fs.rmSync(fixture, { recursive: true, force: true });
}
const result = { version: 1, source_commit: 'e30f0b4b238ee5618928bcd17b4571eb046e7038',
  port: 'Actual SDK query methods; fetch and Git capture return in-memory fixture responses, package manifests in owned directories', rows };
const target = path.join(root, 'fixtures/tools/package-queries-oracle.json');
if (process.argv.includes('--capture')) { assert(!fs.existsSync(target)); fs.writeFileSync(target, JSON.stringify(result, null, 2) + '\n'); }
else assert.deepEqual(result, JSON.parse(fs.readFileSync(target, 'utf8')));
console.log(JSON.stringify({ status: 'passed', package_query_cases: rows.length, external_commands: 0, network_actions: 0 }));
