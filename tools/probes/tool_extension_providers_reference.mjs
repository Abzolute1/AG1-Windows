// Actual SDK registry behavior with an owned models.json and in-memory fixture
// credentials/hooks. No model generation, login, shell expansion or network I/O.
import fs from 'node:fs';
import path from 'node:path';
import assert from 'node:assert/strict';
import { fileURLToPath, pathToFileURL } from 'node:url';
await import('../offline_guard.mjs');
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const { ModelRegistry } = await import(pathToFileURL(path.join(root, 'reference/ag1-executable/packages/coding-agent/dist/core/model-registry.js')));
const { getApiProvider } = await import(pathToFileURL(path.join(root, 'reference/ag1-executable/packages/ai/dist/api-registry.js')));
const fixture = fs.mkdtempSync(path.join(root, '.local/p09-extension-providers-'));
const model = (id) => ({ id, name: id, reasoning: false, input: ['text'], cost: { input: 1, output: 2, cacheRead: 0, cacheWrite: 0 }, contextWindow: 8192, maxTokens: 1024 });
const baselineConfig = { providers: {
  'fixture-a': { api: 'openai-completions', apiKey: 'fixture-key-a', baseUrl: 'https://fixture-a.invalid/v1', models: [{ ...model('base-a'), headers: { 'X-Fixture-Model': 'base' } }] },
  'fixture-b': { api: 'openai-completions', apiKey: 'fixture-key-b', baseUrl: 'https://fixture-b.invalid/v1', models: [model('base-b')] },
} };
const steps = [
  { name: 'fixture-a', config: { hooks: { streamSimple: true } } },
  { name: 'fixture-a', config: { models: [model('invalid')] } },
  { name: 'fixture-a', config: { baseUrl: 'https://fixture.invalid', models: [model('invalid')] } },
  { name: 'fixture-a', config: { baseUrl: 'https://fixture.invalid', apiKey: 'fixture', models: [model('invalid')] } },
  { name: 'fixture-a', config: { baseUrl: 'https://replacement.invalid', api: 'fixture-shared-api', apiKey: 'replacement-key', headers: { 'X-Fixture': 'provider' }, authHeader: true,
    hooks: { streamSimple: true }, models: [{ ...model('new-a'), headers: { 'X-Fixture': 'model' } }, { ...model('new-a2'), api: 'openai-responses', thinking: { supported: true }, compat: { supportsStore: false }, headers: { 'X-Old': 'retained until refresh' } }] } },
  { name: 'fixture-a', config: { baseUrl: 'https://override.invalid' } },
  { name: 'fixture-a', config: { headers: {}, authHeader: false } },
  { name: 'fixture-a', config: { models: [] } },
  { name: 'fixture-b', config: { api: 'fixture-shared-api', hooks: { streamSimple: true } } },
  { name: 'fixture-a', config: { api: 'fixture-shared-api', hooks: { streamSimple: true } } },
  { name: 'fixture-absent', action: 'unregister' },
  { name: 'fixture-b', action: 'unregister' },
  { name: 'fixture-a', action: 'unregister' },
  { name: 'fixture-a', oauthCredential: true, config: { api: 'fixture-oauth-api', baseUrl: 'https://oauth.invalid', oauth: { name: 'Fixture OAuth' }, hooks: { oauth: { modifyModels: true } }, models: [model('oauth-a')] } },
  { name: 'fixture-b', config: { baseUrl: 'https://other-override.invalid', headers: { 'X-Fixture': 'other' } } },
  { name: 'fixture-b', action: 'unregister' },
  { name: 'fixture-a', action: 'unregister' },
];
const clean = value => JSON.parse(JSON.stringify(value));
let oauthCredential = false;
const hookCalls = [];
try {
  const configPath = path.join(fixture, 'models.json'); fs.writeFileSync(configPath, JSON.stringify(baselineConfig));
  const registry = ModelRegistry.create({ getOAuthProviders: () => [], get: name => oauthCredential && name === 'fixture-a' ? { type: 'oauth', access: 'fixture-access', refresh: 'fixture-refresh', expires: 12 } : undefined }, configPath);
  assert.equal(registry.getError(), undefined);
  const snapshot = () => clean({ models: registry.getAll().filter(m => m.provider.startsWith('fixture-')),
    requestConfigs: Object.fromEntries(registry.providerRequestConfigs), modelHeaders: Object.fromEntries(registry.modelRequestHeaders),
    registered: [...registry.registeredProviders.keys()], streamOwners: Object.fromEntries(['fixture-shared-api', 'fixture-oauth-api'].flatMap(api => {
      const provider = getApiProvider(api); return provider ? [[api, provider.streamSimple({ api }, {}, {}).fixtureOwner]] : [];
    })), hookCalls: [...hookCalls] });
  const baseline = snapshot(), rows = [];
  for (const input of steps) {
    if (input.oauthCredential !== undefined) oauthCredential = input.oauthCredential;
    let error;
    try {
      if (input.action === 'unregister') registry.unregisterProvider(input.name);
      else {
        const config = structuredClone(input.config), hooks = config.hooks; delete config.hooks;
        if (hooks?.streamSimple) config.streamSimple = () => ({ fixtureOwner: input.name });
        if (hooks?.oauth?.modifyModels) config.oauth.modifyModels = (models, credentials) => {
          hookCalls.push({ name: input.name, access: credentials.access, ids: models.filter(m => m.provider.startsWith('fixture-')).map(m => m.id) });
          return models.map(m => m.provider === input.name ? { ...m, name: m.name + ' via OAuth' } : m);
        };
        registry.registerProvider(input.name, config);
      }
    } catch (caught) { error = caught.message; }
    rows.push({ input, ...(error ? { error } : {}), state: snapshot() });
  }
  const result = { version: 1, source_commit: 'e30f0b4b238ee5618928bcd17b4571eb046e7038', baselineConfig, baseline, rows };
  const target = path.join(root, 'fixtures/tools/extension-providers-oracle.json');
  if (process.argv.includes('--capture')) { assert(!fs.existsSync(target)); fs.writeFileSync(target, JSON.stringify(result, null, 2) + '\n'); }
  else assert.deepEqual(result, JSON.parse(fs.readFileSync(target, 'utf8')));
  console.log(JSON.stringify({ status: 'passed', extension_provider_registry_steps: rows.length, fixture_credentials_only: true, network_actions: 0, model_actions: 0 }));
} finally { fs.rmSync(fixture, { recursive: true, force: true }); }
