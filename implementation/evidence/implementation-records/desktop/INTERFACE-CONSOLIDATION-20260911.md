# Model-facing interface consolidation — 2026-09-11

> Historical evidence, archived 2026-09-13. Status, commands, permissions and paths below describe their original checkpoint. For current work start with [implementation status](../../../README.md). Original bytes and old paths are preserved in the [relocation manifest](../manifest.json) and recovery archive.

Status: **installed and verified**, release `/opt/ag1/releases/1d7bca72efe74a778881314d15f3c77f`. This follows Luna's report that the earlier reliability improvements still left too many overlapping interfaces. It changes the default tool surface and native routing, rather than merely adding instructions.

## Delivered behavior

New default selections expose `inspect` for file reads, literal searches with surrounding lines, and filename discovery; `work` for current intent/claims and runtime observations; `evidence` for retained evidence retrieval; and `ui` for owned-preview capture, interaction or claim verification. `preview` still owns the preview lifecycle. Each facade is available only when its native dependencies exist.

`work.observe` binds observations to the current task and projects the task-state update atomically. Tool receipts retain evidence automatically. `memory` remains the distinct place for useful knowledge beyond the task. Optional todo/episode controls are no longer part of the default surface, avoiding mandatory parallel bookkeeping.

Evidence retrieval, retention and release have separate typed tools: `evidence`, `retainEvidence`, `releaseEvidence`. Existing immutable artifacts, bounded reads, provenance, cursors and scoped leases remain the backend. Retrieval accepts returned continuation arguments directly. Matching later native verifiers still control automatic lease release.

`capability` discovers specialist tools and their schemas, and executes a fixed specialist allowlist through the original native adapters. This retains legacy source capsules, raw artifact operations, checklist/history views and advanced edit/UI controls without advertising each as a competing default. `edit` remains the ordinary editing interface. Existing saved explicit/legacy tool selections are preserved exactly: **restart AG1 and start a new chat to get the smaller default surface**. The number of tools depends on available host capabilities; the packaged fixture exposed 18.

Tools now carry structured descriptive effects, including a schema `$comment` visible through the tested provider transports. Capability discovery reports active policy, granted folders and shell sandbox boundaries; schema discovery with arguments can report a policy decision. These are informational, not approval grants. Actual execution still validates canonical arguments and prepared targets and uses normal approval policy. Shell enforcement was not weakened or redesigned; its report explicitly distinguishes OS-enforced boundaries from sandbox-off operation.

## Native identity and CV preservation

The transcript retains the tool name the model actually called. Native receipts retain the original canonical operation and arguments. Journal publication, state ownership, approvals, source provenance and verifier attribution independently reconcile those identities. Durable session resume is tested. CV classification recognizes the native operation behind facade results; its thresholds and budgets are unchanged. The exact compiler change was compared against the saved pre-pass file and consists only of canonical-name lookups. Local model/runtime profiles and the previously accepted process helper remain unchanged.

## Validation and limits

23 focused Rust tests passed: prompt/surface contracts, native routed workflows, denied specialist edits, legacy read behavior, composite IDs, observation projection including fault handling, artifact authority/retrieval, preview-target authority, the 16-capture frozen CV comparison, and independent conversations/approval scope/resume.

The production build passed. The packaged CLI completed an isolated scripted-provider workflow with nine generations and eight native tool calls, zero errors, original transcript identities, visible effects metadata, and unchanged fixture source. This exercised default selection without manually selecting tools. A separate real `gpt-5.6-luna` check completed in 10.43 seconds: Luna independently selected `inspect`, read the owned file with zero tool errors, and correctly identified `work`, `evidence`, and `preview` followed by `ui`. It created no work records for the simple question. This is bounded discoverability evidence, not a long-horizon quality benchmark or a new GitHub mutation test.

Initial integration attempts exposed an outer-loop facade/canonical identity conflict, a fixture generation limit, and a fresh-client exact-definition mismatch. These were corrected and rerun successfully. Failed logs are retained alongside passing logs. The default-selection projection now only filters tools; it does not mutate exact native definitions.

All 1,142 installed payload files, the installation receipt and protected hashes verified. Previous release `5a3409cbc2a74ca0ac0ecd39d8582ff6` remains retained. Archive SHA256: `c561649d2d69f03fc53ec709ae33ac5550664a4f9710053794a9c11c58fea7b6`. Input SHA256: `071ffed38c5d8ebce32b606145e912d2a3e02aada02f8c4a12b41ddf3d7c6edf`.

No desktop capture or interruption of user-owned app/model processes occurred. The final weekly usage check at 12:36 UTC showed 5% remaining. MCP and unrelated backlog items remain paused. Advanced protocols remain available when explicitly needed; this does not claim that all harness complexity has disappeared.

Evidence directory: `.local/interface-consolidation-20260911/`, including before files, scoped patch, failed/passing logs, packaged requests, live Luna response, CV preservation comparison and installation verification. [Machine summary](../../P14/interface-consolidation-20260911.json).
