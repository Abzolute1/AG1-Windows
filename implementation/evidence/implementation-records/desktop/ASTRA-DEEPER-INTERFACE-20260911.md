# Astra deeper interface fixes — 2026-09-11

> Historical evidence, archived 2026-09-13. Status, commands, permissions and paths below describe their original checkpoint. For current work start with [implementation status](../../../README.md). Original bytes and old paths are preserved in the [relocation manifest](../manifest.json) and recovery archive.

Status: complete, installed systemwide and live verified on 2026-09-11. All six authorized deeper fixes are delivered. Luna completed with no tool errors; Qwen completed after correcting one missing required argument. See the [machine evidence](../../P14/astra-deeper-interface-20260911.json).

The user authorized this work after the surgical pass and reported 9% weekly usage remaining. The last local Codex account check showed **7% remaining** at 04:18 UTC. No subagents or desktop screenshots were used. CV thresholds, local model settings, the accepted process helper and historical binaries were preserved. MCP remains paused at its separate checkpoint.

## Implemented

1. **Verification-scoped leases.** `artifact lease` and `check` share optional `verificationScope`. A successful native check releases only that session's matching automatic leases created strictly before the check's start timestamp. Timestamp ties, unscoped historical leases, other obligations and manual leases remain retained. Scope is read from integrity-checked retained check arguments. Release and completion receipt commit atomically; replay cannot release renewed evidence. Fork/import preserve optional scope metadata without a database migration.
2. **Owned preview lifecycle.** `preview status/start/stop` reuses native Runtime ownership, manifests, installed toolchains, launch approval, source/configuration fences and readiness identity. The bounded start supports an installed root Vite dev script, a strict owned loopback port and a five-minute lifetime. Status discovers the web target and returns a URL only after the process owns the listener and serves the exact manifest. Stop requires the current web-preview operation ID and cannot stop another kind of Runtime target. Other project types remain in the Runtime UI. No arbitrary browser URL or user-desktop capture is introduced.
3. **Consistent editing facade.** `edit mode=preview|apply` uses the same original-file, disjoint replacement semantics. Preview writes nothing and returns `sourceSha256` in both metadata and a visible text receipt; supplying it as `expectedSha256` rejects a later apply if the source changed. This is source binding, not a stored patch-token transaction; use the same edits to apply the preview. Existing exact/fuzzy and legacy editApply/editPreview semantics remain supported and labelled compatibility operations. Folder permissions and publication CAS remain enforced.
4. **Direct evidence recovery.** `artifact action=retrieve` takes a literal query, known ID (optionally with a query), or supplied cursor. It combines lookup and exact text recovery, includes immutable ID/hash/origin/range provenance, and returns executable continuations. `maxBytes` bounds the entire serialized result, including escaping and metadata (2048–32768; default 8192). Catalog recovery includes up to two alternative matches and flags truncated discovery. Advanced search/grep/read remain available. Recovery uses local retained storage, not another provider API request.
5. **Explicit observation projection.** `runtimeObservation taskObjective` associates a single event with the exact current task objective, rejecting a changed objective. Observation and task-state projection commit together. `updateTaskState=false` records evidence only. Untagged legacy calls retain their established relatedness behavior. Recording pass/fail remains a claim, never a forged native verification receipt; todo/history/memory remain optional views rather than mandatory duplicate writes.
6. **Exploration provenance and freshness.** Native child results retain revision references from recorded reads plus child/checkpoint identities. The board carries them and a retained result-artifact ID. Later workers see age, source checks (matching revision, stale, missing, invalid or unverified) and explicit incomplete-coverage labels. Matching hashes do not verify a summary's claim. Freshness inspection is bounded and cached; old boards with no references remain visibly unverified. Shared/isolated delegation and ownership are preserved.

## Verification and retained evidence

**36 unique focused tests passed** (37 executions, including one repeated edit test). The [test manifest](../../../../.local/astra-deeper-interface-20260911/verified-tests.json) records the logs. Coverage includes lease isolation/fork/import/atomic replay, stale edits and folder authority, bounded UTF-8 evidence recovery, observation projections, board provenance/freshness, and a real owned Vite start/ready/stop lifecycle. Completion checks cover real test receipts, zero tests, forged verifier output, later source mutation and hidden failures. This is focused acceptance, not a claim that all unrelated frozen historical oracle contracts pass.

Final live probes used the exact installed CLI in disposable workspaces with six relevant tools (`read`, `edit`, `artifact`, `preview`, `check`, `bash`). Both models previewed and applied an edit with its source hash, recovered exact retained check evidence, leased it under a named scope, ran the matching check, confirmed release, checked preview status and passed a real Node test. Both preserved the protected fixture files and exited 0. The fixture intentionally had no Vite dev script; actual preview startup is covered by the native lifecycle test.

- [Luna final run](../../../../.local/astra-deeper-interface-20260911/final/live-luna-accepted.json): 35.77 seconds, 10 tool calls, zero tool errors.
- [Qwen35B final run](../../../../.local/astra-deeper-interface-20260911/final/live-qwen-accepted.json): 142.44 seconds, 15 tool calls. Qwen omitted the lease's required `reason` once. Native validation rejected the call before effects; Qwen supplied the reason and completed successfully. No valid request failed. The original probe's stricter zero-argument-errors assertion remains false, explicitly preserved in the [recovery audit](../../../../.local/astra-deeper-interface-20260911/final/qwen-final-audit.json); it has not been relabelled as a clean run.

## Installed production checkpoint

- Current release: `/opt/ag1/releases/fab3e5b44e114c51827453cdb3732951`, selected by `/opt/ag1/current`.
- Production profile: `astra-interface-completion-production-20260911`.
- Archive: `.local/p14-packages/0889a606d1504f36b1da64dad21c27ae/ag1-0.1.0-rc-linux-x86_64.tar.gz`.
- Archive SHA256: `b2a6847fb3c74a062e3e2a3173f00beae013011bce784ebf3246bc6357bc3c30`.
- Input SHA256: `1c2119d6bfa17197b17658444d0fa32e2bde4a3492d178afe1af66eb6daaf9cd`.
- All **1,142 installed payload files**, installation receipt and protected hashes verified: [installation report](../../../../.local/astra-deeper-interface-20260911/final/installation-verified.json).

Reopen AG1 normally to use the installed build. The previous release and all earlier candidate/build profiles remain available. The owned live-test runtime has exited; its port 18080 has no listener. No source changes followed the final build; only handoff documentation was updated.

## Validation history

Earlier unsuccessful probes and regression reports remain under `.local/astra-deeper-interface-20260911/`; they are not acceptance evidence. The first Luna fixture incorrectly assumed ordinary read output was already externalized, then needed a real test receipt. The first Qwen fixture invented a hash and reached its generation limit. The preview hash is now visible in the text receipt, remembered previews retain a stale-source fence, and final probes allow 20 generations.

The intermediate installed checkpoint `6680042c36fb47f68515a6dedb4ee766` exposed a separate completion-classifier defect: npm's `@1.0.0 test` banner was interpreted as zero tests despite a passing real test. The final build strips only npm script-header lines before the unchanged zero-test matcher. Regression tests retain rejection of actual zero-test output and unsupported verification claims. The final installed release supersedes this intermediate checkpoint.

## Scope and remaining work

This closes the surgical and six deeper Astra interface improvements. Preview startup remains bounded to installed root Vite dev scripts with a five-minute lifetime; other target types use Runtime UI. Legacy editing and advanced artifact controls remain compatible. Observation projection reduces duplicate task-state recording without merging all history/memory mechanisms. Existing todo scheduling and shared/isolated delegation remain intact. The separate MCP, broader UI and release backlog in the [task index](../../../../tasks/README.md) is not completed by this pass.
