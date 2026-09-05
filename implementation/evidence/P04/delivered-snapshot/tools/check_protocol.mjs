// Development checker: generated TS compilation is a separate required gate command.
import assert from 'node:assert/strict';
import { readFileSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { pathToFileURL } from 'node:url';
import path from 'node:path';

const root = path.resolve(import.meta.dirname, '..');
const [compiled, output] = process.argv.slice(2);
if (!compiled || !output) throw new Error('usage: check_protocol.mjs COMPILED_CHECK OUTPUT_JSON');
const require = createRequire(path.join(root, 'reference/ag1-executable/package.json'));
const Ajv = require('ajv/dist/2020.js');
assert.equal(require('ajv/package.json').version, '8.18.0');
assert.equal(require('typescript/package.json').version, '5.9.3');
const schema = JSON.parse(readFileSync(path.join(root, 'product/crates/ag1-protocol/schema/protocol.schema.json'), 'utf8'));
const ajv = new Ajv({ strict: true, allErrors: true, formats: { uint16: true, uint32: true } });
const validate = ajv.compile(schema);
const { roundTrip } = await import(pathToFileURL(path.resolve(compiled)).href);
const fixtures = roundTrip();
assert.ok(fixtures.length >= 22, 'empty or incomplete fixture selection');
let peak = 0;
for (const fixture of fixtures) {
  assert.ok(validate(fixture), JSON.stringify(validate.errors));
  peak = Math.max(peak, Buffer.byteLength(JSON.stringify(fixture)));
}
const command = structuredClone(fixtures.find((f) => f.type === 'command'));
const invalid = [null, {}, { ...command, extra: true }];
for (const field of ['expected_revision', 'id']) {
  const bad = structuredClone(command); bad.data[field] = 123; invalid.push(bad);
}
for (const decimal of ['00', '-1', '1e9', '1'.repeat(21)]) {
  const bad = structuredClone(command); bad.data.expected_revision = decimal; invalid.push(bad);
}
const missing = structuredClone(fixtures.find((f) => f.type === 'snapshot'));
delete missing.data.last_terminal; invalid.push(missing);
for (const fixture of invalid) assert.equal(validate(fixture), false, `schema accepted ${JSON.stringify(fixture)}`);
assert.ok(peak < 64 * 1024);
writeFileSync(output, JSON.stringify(fixtures, null, 2) + '\n');
console.log(JSON.stringify({ roundtrips: fixtures.length, schema_rejections: invalid.length, max_fixture_bytes: peak, typescript: '5.9.3', ajv: '8.18.0' }));
