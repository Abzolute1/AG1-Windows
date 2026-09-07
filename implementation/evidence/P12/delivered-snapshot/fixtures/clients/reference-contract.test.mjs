// Execute pinned, unmodified stdio reference behavior in an owned child only.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, mkdtempSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';
import ts from '../../product/apps/desktop/node_modules/typescript/lib/typescript.js';
const reference = new URL('../../reference/ag1-executable/', import.meta.url);
const text = readFileSync(new URL('packages/coding-agent/src/core/mcp-client.ts', reference), 'utf8');
const temporary = mkdtempSync(join(tmpdir(), 'ag1-p12-mcp-'));
writeFileSync(join(temporary, 'mcp.mjs'), ts.transpileModule(text, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ES2022 } }).outputText);
const { McpClient, mcpToolToAgentTool } = await import(pathToFileURL(join(temporary, 'mcp.mjs')));
process.on('exit', () => rmSync(temporary, { recursive: true, force: true }));
const server = `const rl=require('node:readline').createInterface({input:process.stdin});rl.on('line',line=>{const r=JSON.parse(line);if(r.method==='tools/call'&&r.params.name==='wait')return;let result=r.method==='tools/list'?{tools:[{name:'echo',description:'Owned fixture',inputSchema:{type:'object',properties:{text:{type:'string'}}}}]}:r.method==='tools/call'?{content:[{type:'text',text:r.params.arguments.text},{type:'image'}]}:{};process.stdout.write(JSON.stringify({jsonrpc:'2.0',id:r.id,result})+'\\n');});`;
test('retained stdio reference discovers schema, converts text and disconnects pending calls', async () => {
  const client = new McpClient({ command: process.execPath, args: ['-e', server], timeout: 1000 });
  try {
    const [tool] = await client.connect();
    assert.equal(tool.inputSchema.properties.text.type, 'string');
    const adapter = mcpToolToAgentTool(tool, client);
    assert.equal(adapter.name, 'mcp_echo'); assert.equal(await adapter.execute({ text: 'native reference fixture' }), 'native reference fixture');
    const pending = client.callTool('wait', {}); const rejection = assert.rejects(pending, /disconnected/); client.disconnect(); await rejection;
    await assert.rejects(client.callTool('echo', {}), /not connected/);
  } finally { client.disconnect(); }
});
test('retained stdio startup exit and timeout reject without an HTTP fallback', async () => {
  for (const source of ['process.exit(7)', 'setInterval(()=>{},1000)']) {
    const client = new McpClient({ command: process.execPath, args: ['-e', source], timeout: 150 });
    try { await assert.rejects(client.connect(), /exited|timed out|disconnected/); assert.equal(client.isConnected(), false); } finally { client.disconnect(); }
  }
});
test('independent Lit, Slack and pods manifests retain TS APIs and the runner remains removed', () => {
  for (const name of ['web-ui', 'mom', 'pods']) {
    const packageJson = JSON.parse(readFileSync(new URL(`packages/${name}/package.json`, reference), 'utf8'));
    assert.equal(packageJson.name, `@ag1/${name}`); assert.ok(packageJson.dependencies[name === 'pods' ? '@ag1/agent-core' : '@ag1/ai']);
  }
  const barrel = readFileSync(new URL('packages/coding-agent/src/core/auton/index.ts', reference), 'utf8');
  assert.ok(barrel.includes('has been removed')); assert.ok(!/export[^;]*from\s+["']\.\/runner/.test(barrel));
  const dispositions = JSON.parse(readFileSync(new URL('./dispositions.json', import.meta.url), 'utf8')).rows;
  for (const id of ['A82', 'A83', 'A84']) assert.equal(dispositions.find(row => row.id === id).disposition, 'reference-client');
});
