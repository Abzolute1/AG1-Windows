import { fixtures } from './roundtrip.js';
import type { Command, Sequence, Snapshot, WireMessage } from '../../product/apps/desktop/src/generated/protocol.js';

// These assertions must fail compilation if generated types weaken into any/number.
// @ts-expect-error Decimal sequences cannot be JavaScript numbers.
const lossySequence: Sequence = 9007199254740993;
// @ts-expect-error Unknown command tags are rejected.
const invented: Command = { kind: 'silently_execute_everything' };
// @ts-expect-error Required nullable is different from optional.
const missingTerminal: Snapshot = { fake: true, scope: { workspace: 'w', session: 's', instance: 'i', epoch: '1' }, branch: 'b', revision: '0', watermark: '0', queued: [] };
void lossySequence; void invented; void missingTerminal;

function check(value: unknown, message: string): asserts value {
  if (!value) throw new Error(message);
}

export function roundTrip(): WireMessage[] {
  const encoded = JSON.stringify(fixtures);
  const decoded: WireMessage[] = JSON.parse(encoded);
  check(JSON.stringify(decoded) === encoded, 'JSON roundtrip differs');
  const commands = decoded.filter((message) => message.type === 'command');
  check(!Object.hasOwn(commands[0].data, 'extension'), 'absent custom field was inserted');
  check(Object.hasOwn(commands[1].data, 'extension') && commands[1].data.extension === null, 'explicit null was lost');
  check(JSON.stringify(commands[2].data.extension) === JSON.stringify({ decimal: '18446744073709551615', future: [null, { flag: true }] }), 'custom payload changed');
  const cursors = decoded.filter((message) => message.type === 'cursor');
  check(cursors[0].data.after === '9007199254740993', 'sequence was rounded');
  check(BigInt(cursors[1].data.after) === 18446744073709551615n, 'u64 maximum was rounded');
  return decoded;
}
