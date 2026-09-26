# Local thinking controls restored — 2026-09-11

> Historical evidence, archived 2026-09-13. Status, commands, permissions and paths below describe their original checkpoint. For current work start with [implementation status](../../../README.md). Original bytes and old paths are preserved in the [relocation manifest](../manifest.json) and recovery archive.

Installed release: `/opt/ag1/releases/c732fa9ff5074ac1bb126491f0024116`.
Restart AG1, click the model name in the composer, choose Thinking/effort, then send. The model does not need to be loaded first.

## Cause and repair

The toggle component still existed. The configured local model entries had no persisted thinking profiles, so discovery depended on that exact model being loaded alone at the endpoint. While unloaded, sleeping, or another model was selected, the picker received unknown capabilities.

Read the actual GGUF tokenizer.chat_template metadata without loading weights or starting inference. Restored model-bound profiles in the private desktop native-host.json:

- Qwen3.6-35B-A3B-UD-Q4_K_XL: Off / On; default On.
- Qwen3.8-27B-UD-Q4_K_S: Off / Low / Medium / Extra high; default Extra high.
- Qwen3.8-Flash-Next-AD-3.84bpw-IQ4_XS-M64: same four choices; default Extra high.

The 27B template treats high as an alias of xhigh, so it is not exposed as a duplicate effort. Both 3.8 templates explicitly support enable_thinking=false and their actual effort enum is low, medium, xhigh.

Declared model capabilities now take precedence over live heuristic discovery. Discovery remains available for local models without declared profiles. Declared apply/nativeValue metadata survives normalization, and the desktop reasoning action forwards declared native effort profiles to the session, including existing chats. This prevents a UI-only slider whose setting never reaches the backend.

Only the three local models' thinking metadata changed in native-host.json. Model paths, runtime capture, token/context budgets, credentials configuration and other settings remain unchanged. A private before-copy is retained. Other/new local models still need their own verified metadata; no name-based inference or universal effort menu was introduced.

## Verification

16 focused tests passed: 4 capability units, 3 picker tests, 7 provider request contracts and 2 native session thinking contracts. Ten additional exact configured model/mode payload checks confirmed boolean enable_thinking for all choices and reasoning_effort for the two 3.8 models. These checks consumed no inference. The local llama-server parser preserves enable_thinking=false while forwarding the requested effort, consistent with the extracted templates.

Production package built, archive validated, all 1,144 installed files/receipt and protected hashes verified. No user window capture or forced restart. Prior stream/cancellation repair is included.

Private evidence: `.local/local-thinking-restore-20260911/` (template extracts and hashes, implementation.patch, local-models.json, exact-wire.json, test/build logs, native-host-before.json, installation-verified.json). Temporary probe example removed from product sources. Current weekly usage: 2% remaining. Broader backlog remains paused.
