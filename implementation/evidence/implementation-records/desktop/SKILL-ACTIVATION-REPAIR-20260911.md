# Native skill activation repair — 2026-09-11

> Historical evidence, archived 2026-09-13. Status, commands, permissions and paths below describe their original checkpoint. For current work start with [implementation status](../../../README.md). Original bytes and old paths are preserved in the [relocation manifest](../manifest.json) and recovery archive.

Installed and verified: `/opt/ag1/releases/75445b5e02cf4f60a4fd74718d6f4251`.

The native prompt advertised automatic skill embedding, but its matcher was called only by tests. Explicit `/skill:name` merely told the model to read a path. The harness now carries host-captured skill bodies alongside prompt configuration and actually expands matching user instructions before native message delivery.

Automatic selection uses the existing deterministic name/trigger matcher and respects `disable-model-invocation`. Explicit `/skill:name` and the desktop Use action include manual-only skills. The Use action captures current resources; normal automatic matching uses the session's captured resource configuration. Unknown explicit skills return a clear error. Skill blocks include their name, location and reference directory, omit YAML frontmatter, and are not expanded twice. Existing task canonicalization keeps the user's objective separate from the embedded skill instructions. Captured bodies survive workspace snapshot updates.

The shared native input path handles plain text, text artifacts, attachment messages, steering and follow-ups. Image blocks are preserved. Ordinary large messages preserve the previous input admission behavior; automatic matching skips text larger than 1 MiB and expanded skill prompts are bounded at 1 MiB.

## Verification

Five tests passed: one core activation contract (automatic/manual-only/unknown/unrelated/idempotency/task identity/large input), one actual native desktop Host-to-loopback-provider roundtrip, and three existing resource creation/discovery contracts. The native roundtrip captures four real outgoing request bodies: automatic activation, explicit Use action, unrelated text, and the desktop upload/message-artifact path with an image. Skill content appears once where expected and the image reaches the provider payload. No real model inference was used; these checks verify delivery, not model obedience.

Production build, archive validation and all 1,144 installed file/receipt checks passed. Protected model/CV/runtime configuration hashes verified unchanged. No desktop capture or user process interruption. Weekly usage remains 1%.

Restart AG1 and start a new chat to load the installed implementation and current skill catalog. Skills are instructions, not enforcement of artistic quality or mandatory model behavior. This bounded repair covers activation/loading; it does not replace package management or change folder permissions for referenced files/scripts.

[Machine evidence](../../P14/skill-activation-repair-20260911.json). Scoped patch, tests and packaging receipts are in `.local/skill-activation-repair-20260911/`. The first candidate was superseded after adding large-message compatibility; only the final candidate was installed. Broader MSP/P14 work remains separate.
