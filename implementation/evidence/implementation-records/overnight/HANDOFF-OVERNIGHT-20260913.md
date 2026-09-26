# AG1 overnight investigation — handoff for later today

> Historical evidence, archived 2026-09-13. Status, commands, permissions and paths below describe their original checkpoint. For current work start with [implementation status](../../../README.md). Original bytes and old paths are preserved in the [relocation manifest](../manifest.json) and recovery archive.

**Current instruction: stop here.** The user returned, requested the GPU, stopped the duration run, and then explicitly deferred further fixes until later today. Do not restart inference, an eight-hour run, installation, or substantial implementation merely because an older goal says to continue unattended.

Subsequent questions authorized explanation and a final documentation/evidence check only. The user explicitly said not to run tests. No tests, builds or model calls were run during that final handoff check. Commands and proposed experiments below are for a later explicitly resumed implementation task, not permission to execute them now.

The GPU was released. The test controller, native host, supervisor and Qwen process exited. The final GPU check showed Discord only. No live GitHub resources were mutated for testing.

## Current state at handoff

| Area | State |
|---|---|
| Installed candidate | `/opt/ag1/current` points to `/opt/ag1/releases/7641a402af134e8d9ddb5f4dfade86fd`. This is the validated run8/run9 candidate. |
| New uninstalled change | Tool-artifact timestamp fix plus one passing native regression, in the working tree. **Not packaged or installed.** |
| Latest duration | `.local/p14-long-horizon/overnight-9`, stopped at the user's request after **2.2389903826 hours**. |
| Completed workload | **58 source-verifier passes; 58 manual claim reviews: 57 passed, one semantic-only failure.** |
| Stopped trial | `activities/000083-task` was in progress when the controller was interrupted. It is not counted as a completed/verifier-approved task, even though the native client subsequently drained. |
| Eight-hour claim | **No uninterrupted eight-hour result achieved.** Do not combine earlier attempts or relabel this stopped run as a pass. |
| Cleanup/preservation | Full post-run audit completed before source edits. All 16 observed process identities closed; original historical artifacts and protected runtime materials passed preservation checks. |
| Further work | Deferred by the user. Resume only when asked later today. A new duration is intended for tonight after fixes and validation. |

Evidence below is relative to the repository unless stated otherwise. The main evidence directory is `implementation/evidence/overnight-20260913/` (abbreviated **E/** below).

## Working rules and protected material

- Read `/home/alex/Projects/AG1/github.md` completely when resuming the original implementation brief. It was fully read during this session. `implementation/GITHUB-IMPROVEMENTS-20260913.md` maps delivered behavior and justified architectural deferrals to that brief.
- The original duration requirement is in `phases/P14-release-and-long-horizon-gates.md:29`; formal gate/report requirements are at line 48. Those require both 8h and 24h plus other release gates. The user requested an investigation similar to the eight-hour requirement; completing that investigation alone would not complete formal P14 acceptance. Older P14 handoffs are historical context, and their permission/status statements do not override this conversation's latest instructions.
- Preserve the heavily dirty/untracked workspace. Many changes predate this follow-up. Do **not** reset, clean, stash, blanket-copy an old checkout, or commit/push unrelated work.
- No live GitHub mutations for tests. No new model load while the user needs the GPU. Use CPU fixtures for the pending implementation work.
- Use AG1's existing architecture and tool/policy boundaries. Do not remove legacy canonical tools that restored sessions explicitly select; the preferred surface already groups them.
- Use milestone testing, then appropriate broader validation. Do not rerun unchanged frontend/native suites after each tiny change.
- Build with `CARGO_TARGET_DIR=/home/alex/Projects/AG1/.local/p14-target`. Do not use the default `product/target` for new builds.
- `/tmp/ag1-overnight-followup` is **stale**. Do not merge or rsync it back.
- No subagents were authorized for this work.
- Systemwide installation was previously authorized, but do not store credentials in files or logs. Installation of the new timestamp change is deferred with the remaining work.

Protected files and hashes:

| Material | SHA-256 |
|---|---|
| `/home/alex/.ag1/agent/llamacpp-model-profiles.json` | `aa02688c4d39ef73234f0ec381eed22dbe02463881626423049b75b5a12ad0d0` |
| `product/target/release/ag1-process-host` | `e1c7ca1711b2b7834eea1c99a55fe958cb4cfb880ed3c6909b06232c36bc1da2` |
| `.local/p13-runtime-capture-20260907/configuration.json` | `b6ea6f521e34c0278dd5d3c097ca2c83036eeebd52cb49305c4623734f3a9fba` |

Preserve the user's Qwen profiles, models, llama.cpp binaries, Cruzible state and existing process supervisor. Do not print the entire captured configuration; it contains private encoded inputs. The selected Qwen profile is `Qwen3.6-35B-A3B-UD-Q4_K_XL`, with the original tuned 90,000-context launch plan, MoE CPU offload 20, q8 KV, fit off, 99 GPU layers, batch/ubatch 256/128, 12/12 threads and preserved reasoning settings. Do not retune it to improve a test score.

Previous authorized cleanup recovered roughly 284 GiB of obsolete AG1 build/package material, 71.6 GiB of Steam data and 6.9 GiB of obsolete releases. Steam saves were backed up to `/home/alex/steam-saves-backup-20260913`. See `implementation/evidence/storage-cleanup-20260913/REPORT.md`. The later space check showed about 335 GiB free on `/home` and 8.1 GiB on `/`; further deletion is unnecessary. Do not delete unique P13/P14 history or current receipts.

## Implemented, validated and installed before run9

1. **White inner title bar:** Hyprland detection handles colon-separated `XDG_CURRENT_DESKTOP`; the inner GTK title bar is removed while retaining the compositor bar. `AG1_NATIVE_TITLEBAR=1|0` is an explicit override. Actual Wayland evidence: `E/titlebar-wayland.png`.
2. **GitHub settings polish:** existing theme/components, improved account/permission grouping, tabs, forms, tables, quota and responsive layouts. Twelve visual states at wide/narrow/small widths had no overflow or exceptions. No live GitHub test mutations.
3. **Historical command receipts:** a premature scope guard rejected legitimate completed receipts after session changes. A narrow historical-receipt exemption preserves original-owner/workspace checks; ordinary mutations still require current scope.
4. **Internal working-state transport:** request-local state was becoming a fresh user message after tools, causing Qwen to revive an obsolete historical command. It now joins the last tool result in the provider request while preserving call identity, result content, durable history and current task state.
5. **Ordinary non-Git folders:** source/repository orientation now handles absent `.git` normally. Invalid metadata and parent-repository escape remain rejected.
6. **Small-history context overflow:** many individually small messages bypassed prior reductions and exceeded the working budget. Complete older turns/tool rounds now archive durably after cheaper reductions; current/recent/protected context and leases remain protected. Exact raw custody remains available separately.
7. **Ambiguous preferred work calls:** `work observe` rejects mixed decision fields rather than silently dropping them. `work record` remains the decision path. A pre-fix live journal reopened successfully with the corrected flow.
8. **Custom-verifier completion receipts:** print mode's old transcript naming heuristic could reject actual successful commands such as `node verify.mjs`. Native typed receipts now supply verification authority, while later mutations, failures, zero-test runs, echoed commands and inappropriate alternatives remain rejected. **This is a print-mode completion fix, not a universal ordinary-chat truthfulness guarantee.**

Four measured performance changes are installed:

- CLI wire messages are encoded once. The measured encoding step improved about 55.6%; this is not an end-to-end latency claim.
- Conversation preview formatting works backward within the 100-entry/8,000-byte bounds instead of formatting the entire history. The measured projection step improved about 95.4%, preserving exact count/order and raw journal access.
- A bounded artifact preview cache invalidates on local/external database changes and relevant scope/limit changes; raw cached record admission is capped at 64 KiB. The measured repeated query improved about 98.8%.
- Context custody drops temporary full-journal/normalized/base64 buffers before reduction and checks the final revision without reloading an unused full snapshot. A native allocation probe reduced overlap at reducer entry from 15,249,359 bytes to 248 bytes. Initial serialization peak remained unchanged; this does not prove a whole-harness leak fix.

Implementation/measurement references: `E/performance-followup-manifest.json`, `E/followup-merge-result.json`, `E/post-run7-additional-changes.json`, `E/OPTIMIZATION-NOTES.md`, `E/context-custody-allocation-assessment.json`. The nine-file performance merge has a retained pre-merge archive; later validated fixture and completion changes are separately accounted for.

## The new timestamp fix — working tree only

**Bug:** `FileRunner` is cached per session and held its construction timestamp. Different tool artifacts from later turns received that old `createdAt`, making catalog chronology and audit timelines misleading. Approval/repetition expiry uses a separate live clock and was not implicated.

**Changed files:**

- `product/crates/ag1-platform/src/session/files.rs`: `execute_one` now uses `current.selection.journal.timestamp()` when a journal exists, retaining `self.timestamp` for direct invocations without one.
- `product/crates/ag1-platform/tests/session_contract.rs`: `shared_native_runner_dates_tool_artifacts_by_their_actual_turn` uses a real shell, one shared runner, two explicit turn timestamps and distinct output artifacts. It checks both dates, producer identity, first-record immutability and newest-first ordering.

The regression failed before the fix with the old runner date `2026-09-06T00:00:00.000Z`, then passed after the change. Evidence:

- `E/timestamp-regression-red.log`
- `E/timestamp-regression-green.log`
- `E/timestamp-followup.json`
- `E/timestamp-followup.patch` — exact follow-up against the pre-stop baseline, avoiding unrelated root Git differences
- `E/post-stop-source-before.tar.gz` and `.json` — backup/hash inventory of 337 relevant source/fixture files

Only the two files above differed from that backup at handoff. Other reviewed `ContextStore` call sites read existing artifacts or concern background notices/quiescence/extension callbacks without a current turn journal. They were not mechanically changed. No existing artifact metadata was rewritten.

**Remaining validation:** run the complete native session target at a suitable milestone, then package/install with the other fixes. The focused regression alone has passed; do not claim the previous 261-test full result includes this new change.

From `product/`, the completed focused command was:

```bash
CARGO_TARGET_DIR=/home/alex/Projects/AG1/.local/p14-target CARGO_BUILD_JOBS=6 cargo test --release -p ag1-platform --test session_contract shared_native_runner_dates_tool_artifacts_by_their_actual_turn -- --exact --test-threads=1
```

## Remaining fix and optimization queue

### 1. Prompt-prefix invalidation from changing verifier guidance

`E/run9-prompt-cache-finding.json` binds actual provider usage, compiled requests and backend log bytes. Trials 33–36 reported `cacheRead=0` on each first generation, with substantial reuse within later tool rounds. Alternating canonical verifiers changed the system prompt near character 2357 despite hundreds of identical history messages. Two backend requests processed 34,310/35,510 uncached tokens in 96.10/99.47 seconds.

Source: `ag1-core/src/context/projections.rs::turn` builds the contract; `ag1-platform/src/session/context.rs` appends it to `configuration.system_prompt` before history. This is consistent with early-prefix invalidation, **not a model-weight reload**. Stable-verifier ordinary chat was not measured by this comparison.

No fix has been applied. Investigate moving changing verifier guidance into the existing request-local tail while keeping the system prefix stable and native canonical-verifier enforcement intact. Preserve instruction authority and provider compatibility; do not introduce an unsupported late system/developer message or recreate the prior fake-user-turn bug. The existing `context::append_working_state` and provider conversion preserve a last tool result's identity. Add a two-turn request comparison proving stable system/history prefix, current verifier guidance, no journal persistence and no stale verifier carryover. Relevant existing tests are in `session_verification/mod.rs` and `session_print/mod.rs`; several currently assert the contract is in the first system message and need intentional placement-aware updates. Live cache/quality gains remain for tonight.

### 2. Redundant verifier calls and misleading model claims

Qwen sometimes reran a successful verifier two to four times after one edit. A short success result can be followed by roughly 4.4 KiB of task/inspection guidance, including stale-source warnings; staleness after editing does not itself invalidate an executed passing verifier. Current-run repeats occurred at task indices 25, 27 and 42. Their final claims were supported; extra calls are retained efficiency findings. No extra completion-recovery custom message was observed for those comparisons, so causation is not established.

Consider concise request-local status derived from `session/completion_evidence.rs::Evidence` rather than more repeated generic instructions. The existing ledger tracks mutation and actual verification outcomes. Preserve invalidation after later mutations, negative/zero-test/uncertain results and trusted command selection. Do not suppress legitimate deliberate verification or fabricate receipts.

The later discussion clarified that these were **short repetitions that eventually ended**, not observed endless loops. Qwen selected the repeated calls after receiving success; confusing harness guidance is a plausible contributor, not an established sole cause. Use model-independent execution facts and clearer state, rather than Qwen-specific prohibitions. Retain full evidence and legitimate repeat checks. No GPT-6-in-AG1 comparison was run, and no improvement rate or immunity for any model is established. When testing is authorized again, assess correctness, missed verification, false completion and redundant calls across the local model and other models the user uses; preserve comparable workloads/tool surfaces and distinguish latency from quality.

Separate semantic failures include an incorrect explanation of the old clamp expression, missing negation when describing a batch guard, and an earlier false claim of editing/testing without either action. One earlier retry implementation accepted `bad4` although its original verifier passed. Native gates and better guidance can reduce these failures; **do not claim they guarantee model truthfulness or complete boundary coverage.** Keep actual final-output reviews and original verifiers.

### 3. Persistent custody disk growth

Full changing `context-original-journal`, `runtime-request-input` and `context-original-request` snapshots dominate retained bytes. Run7 reached roughly 1.46 GB of artifact payload after about 100 minutes. Context virtualization bounds model input, not all durable copies. Whole-blob addressing cannot deduplicate an appended full snapshot.

`E/custody-compression-probe.json` measured 77–86% lossless zlib savings on a small sample, with exact hash-checked reconstruction. Compression is **not implemented**. A compatible storage redesign must preserve logical raw digests, byte windows, streaming search, import/export, corruption detection, bounded decompression, crash durability and GC roots. Do not overwrite raw blobs with compressed bytes under existing assumptions.

Relevant code: storage `persistence.rs::{publish,verify_blob,blob_bytes,blob_window,blob_path,inspect_integrity}`, `artifact_catalog.rs::scan_artifact`, `artifacts.rs::{put_artifact,collect_orphans}`, and `import.rs` export/recovery paths. Public `blob_path` and some tests/tools directly read raw paths. The store currently identifies schema version 1. `flate2` is already transitively locked, but storage has no direct compression dependency. No format, schema, dependency or reader changes were made during handoff preparation.

Choose a versioned lossless design and validate old-store reads, exact logical-byte reconstruction, truncated/corrupt/oversized inputs, search across decode boundaries, publication failures, archive restoration and collection of unreferenced physical data. A physical compression representation or shared immutable snapshots needs an explicit compatibility plan, not only a favorable compression ratio.

### 4. Valid Git repositories missing optional info directories

System Git accepts a repository without `.git/info` or `.git/objects/info`; AG1's read-only wrapper returns `NotFound`. Evidence: `E/optional-git-info-diagnostic.log` and `E/optional-git-info-case.rs`. The fixture is retained diagnostic source, not an already-merged fix.

`ag1-platform/src/git_read.rs::run` unconditionally pins both paths, binds the common Git directory read-only, then overlays safe info directories to neutralize unsafe metadata such as alternates/attributes. Simply skipping pins fails because bubblewrap cannot create missing mount points below a read-only root. Do not create directories in the user's repository from inspection, bypass the namespace, or call it non-Git.

Installed bubblewrap advertises `--overlay-src`, `--tmp-overlay` and `--ro-overlay`. These are **an untested design option**, not an implemented or proven fix. Preserve pinned-directory/linked-worktree identity and reciprocal-pointer checks, network isolation and read-only process policy. Test normal and missing-directory repositories, linked worktrees, symlink/race containment, unchanged host contents and existing malicious-metadata cases.

### 5. Renderer long-history scaling

The production synthetic `NativeConversation` benchmark mounted 2,000 turns in 1.48–1.75 seconds, appended in roughly 9–12 ms and retained about 62,032 DOM elements/~36 MiB JS heap. This is GPU-disabled Chromium, not native WebKit latency or proof of a leak. Native initial history is bounded; the stress case represents extensively loaded history.

`NativeConversation.tsx` retains loaded messages/turns and renders every turn. A virtualization or rendering-containment change must preserve expanded/hydrated messages, thinking toggles, streaming, scroll anchors, loading earlier history, search and accessibility. Benchmark before/after with the same fixture. Existing scripts: `E/conversation-production.mjs`, `.json`, `.png`, and the earlier development benchmark. Their temporary server/source paths must be checked before reuse. No renderer performance change was started after stopping the run.

### 6. Additional hypotheses, not diagnosed bugs

`WorkingSet` still hashes full prefixes and clones compiled JSON; context preparation also copies branch/messages. These may matter at larger histories, but require measurement and a revision-aware invalidation design. Do not weaken branch/owner/source freshness checks on speculation.

Preferred/default tools already group legacy APIs behind `inspect`, `work`, `evidence`, `ui` and `capability`. Removing canonical tools would break explicit restored selections. Preferred-tool pilot findings include extra observation bookkeeping and inaccurate model call counts; default-surface eight-hour coverage remains absent. Historical archive wording also names legacy retrieval tools; preferred-surface wording should be reviewed without claiming an observed unknown-tool failure.

One concrete wording gap was missing from the first handoff: `product/crates/ag1-core/src/context/projections.rs::task_with_tool` still says to update with `taskState` when the selected preferred control tool is `work`. Its current availability input is a boolean, so it cannot distinguish the two names. The pilot did not execute an unknown tool, but the hint conflicts with the actual route. Carry the selected control-tool identity through the existing projection API and preserve legacy/direct callers when addressing it. `E/preferred-pilot-assessment.json` and `.local/p14-long-horizon/overnight-preferred-pilot-2/` retain the pilot evidence, including the actual 31-tool/13-advanced catalog versus the model's incorrect 27/10 report.

GitHub OAuth/Enterprise/account-switching, durable policy, ETag/sync/webhook work has explicit architectural deferrals in the original brief and acceptance mapping. Do not silently represent a broad endpoint catalog as dedicated UI for every endpoint.

## Validation already completed for the installed candidate

- 266 desktop tests and the TypeScript/Vite production build passed. Earlier GitHub-specific work included 109 tests and 12 visual states.
- All 261 native session contracts passed before run9, including the custom-verifier completion regressions.
- The initial broader Rust invocation had **1,253 passed, 79 failed, one ignored**. After diagnosis/corrections, all 12 affected targets passed complete-target follow-ups, totaling 659 successful follow-up executions. This is **not** one all-green workspace invocation. `E/run8-validation-accounting.json` records the accounting.
- Corrections included current UI/guidance/provenance expectations, profile-relative helper paths, real project fixture construction, deterministic stream ordering and documented serial MCP tests. Production resource limits and original fixture captures were preserved.
- Supporting milestones covered context/continuation/custody/cache invalidation, client receipts and command-completion negatives. Details are in `E/REPORT.md` and retained logs.

When broadening later, use serial runtime/MCP tests and existing fixture prerequisites. `.local/p14-target/release/deps/ag1-process-host` was staged for test helpers; see `E/test-helper-layout.json`. The native project fixture is `project-native-fixture`; build it with the required visual-test feature separately from production packaging. Do not blindly run the older `E/final_validation.py`, which uses assumptions unsuitable for the corrected serial/prerequisite workflow.

## Duration evidence and truthful limits

Run9 started **08:12:55 UTC /10:12:55 Oslo** and its recorded workload stopped after **2 hours 14 minutes**. The raw result is `failed` with `KeyboardInterrupt()` because the user requested stopping; do not rewrite it. `E/run9-user-requested-stop.json` explains the interruption. The attempted escalation to terminate the native host found it had already drained, so **no extra native-host signal was sent**.

Completed run9 milestones:

- Original 2,497,717-token seed; all 64 original historical artifacts retained.
- Branch/fork/resume/checkpoint coverage and actual WebKit reload/reattachment.
- Six new-chat/return cycles retained model PID 1048892 with zero reloads, taking about 0.158–0.171 seconds. This uses actual React handlers through WebKit DOM, not physical mouse automation. `E/run9-desktop-continuity.json`.
- One-hour corrupt database refusal (`NotADatabase`), retained damaged bytes and native archive restoration; subsequent original repairs passed. No runtime authority transferred and no runtime actions executed by restoration.
- At two hours, owned model PID 1051666 was killed while native host PID 1051604 remained alive. The next original repair/verifier passed through normal runtime preparation. `.local/p14-long-horizon/overnight-9/model-recovery.json` and `E/run9-two-hour-milestone.json`. This was between repairs, not an interrupted-generation test.
- First live history archive: 239 frames, 48,375 → 37,284 tokens. Hashes verified; 238 frames matched the previous compiled request exactly and in order, plus one generated boundary marker. `E/run9-first-history-archive.json`. Later resume checkpoints produced replacement archives from durable history; no live chained-archive claim was established.
- Final qualified native-host slopes were **6.27 MiB/hour** (22 post-warmup samples/21.10 minutes) and **4.71 MiB/hour** (43 samples/42.21 minutes), below the fixed 8 MiB/hour budget for these observed lifetimes. `E/memory-run9-stopped.json`. An earlier short-window 9.52 MiB/hour failure remains saved. Short lifetimes and host absence are retained, never treated as zero growth. This is not an eight-hour leak verdict or an isolated before/after leak-fix comparison.

Memory policy: each process identity and the workload receive a fixed 1,800-second warmup; at least ten post-warmup samples qualify a lifetime. The metric is a glibc in-use-or-retained estimate including allocator metadata/thread caches. Owned-tree PSS includes the model and is separate from native-host heap; global GPU readings also include unrelated applications. `E/run9-process-memory-composition-early.json` is one snapshot showing substantial anonymous/shared model memory, not proof that growth is file-backed page cache. Run7 and run9 differ in mode, prompt guidance, task/output rate and interruption history; comparing their totals does not isolate the installed performance fixes.

The four-hour host crash was **not reached in run9**. An earlier short pilot exercised owned host/model crashes, but it is not duration acceptance. The duration explicitly used eight legacy tools: `read`, `grep`, `ls`, `bash`, `edit`, `write`, `artifact`, `sourceContext`. Preferred-tool coverage was a separate pilot. No matched TypeScript/Rust preflight, 24-hour result, cross-platform/signing acceptance or clean overall semantic-quality pass is claimed.

Important prior attempts: run6 exceeded context budget; run7 stopped after 1.9866 hours on false completion without an edit/verifier; run8 stopped after about 21.6 minutes because the external review exporter emitted absolute paths against canonical relative references. That exporter is fixed; original exports and failed results remain retained. Other early attempts found the receipt bug, internal-state replay and strict shared-GPU admission. Detailed chronology is in `E/WORKLOG.md`.

Run9's sole failed semantic review is `.local/p14-long-horizon/overnight-9/claim-reviews/000002.json`: its code/verifier passed, but it falsely described the old clamp's above-range result. With ordered bounds the old expression always returned the maximum. Do not confuse this with the earlier run7 batch-report defects or its false edit/verifier claim. The readable controller log is `/tmp/ag1-overnight-run-9.log`; durable receipts under the run directory remain the authoritative handoff evidence if `/tmp` is cleared.

Final run9 preservation: `.local/p14-long-horizon/overnight-9/post-duration-preservation/result.json`. Final preservation receipt SHA-256: `62701dbcd69447000f17800ebfb19208c7c46f3fa88a57a33beb269a91229cc4`. Raw duration-result SHA-256: `9a8626a1739f85e4943c8d4c19a0d4203ebca2950eae8101d5b2630f84844946`.

## Package and future restart procedure

Installed archive: `.local/p14-packages/7bc153db51e44326818f120e01d9d8e6/ag1-0.1.0-rc-linux-x86_64.tar.gz`, SHA-256 `f41e3db0eec49bcb2abba77394178952857cb3eecc05e1eef16a9fa31761431d`. Input digest `e6ef65207234289ad92d38760bf5a2a5bd8722e046f58d9a4370c62de4adf49d`. `E/package-run8.json` and `E/installation-run8.json` bind all 1,144 installed manifest files. The user's pre-existing desktop process was not forcibly restarted.

Frozen run8/run9 helper binaries are in `.local/overnight-20260913-run8-binaries`; production packaging excludes `visual-test`, and instrumented helpers are built/copied separately. `E/run8-candidate-preparation.json` binds them. They represent the **old candidate**, not the new timestamp source.

When asked to resume:

1. Read this handoff, the brief and exact current source/patch state. Keep the stopped run and backups intact.
2. Finish the agreed fix queue with CPU-only milestone tests. Preserve tested semantics and document any unresolved design limitations explicitly.
3. Validate the complete affected native targets, then appropriate broader checks. Re-run frontend validation only if renderer/UI code changes.
4. Produce a new unique package and helper directory after final validation; do not overwrite run8 receipts or reuse stale helpers as though rebuilt. `tools/release_package.py` provides packaging; system installer is `product/release/install-systemwide.py`. Node toolchain: `.local/toolchains/node-v24.14.0-linux-x64/bin`.
5. Install the validated candidate if still within the user's resumed scope. Confirm installation via its manifest, not merely a successful archive build.
6. Start a **new** output directory and continuous eight-hour clock tonight when the user authorizes GPU use. Do not resume the stopped controller or alter its timestamps. Reassess preferred-surface coverage and whether the run should remain the explicitly scoped print-mode investigation.

The previous command shape was:

```bash
systemd-inhibit --what=sleep --mode=block --who=AG1-overnight --why='User-authorized AG1 harness stress investigation' python3 tools/overnight_print_stress.py NEW_OUTPUT --seed-source .local/p14-long-horizon/live-5 --package-report NEW_PACKAGE_RECEIPT --binaries NEW_HELPERS --heap-library NEW_HELPERS/heap-observer.so --hours 8
```

`tools/overnight_print_stress.py` retains the original source/verifier/recall/context criteria and uses the native print gate for CLI repairs; the WebKit trial remains ordinary chat. `tools/overnight_stress.py` owns preservation/custom investigation behavior. Do not lower limits or weaken verifiers to keep the clock running. Shared GPU permission was set only in disposable configuration to leave Discord untouched; endpoint/runtime custody remains exclusive.

For future manual evidence review, `E/review_pending.py RUN --compact` displays exact visible-execution differences and the complete current final answer. It makes no quality decision. `E/record_review.py` saves **supplied manual decisions**, binds actual records/verification hashes and uses canonical references. Do not auto-pass unseen claims. Failed semantic-only reviews remain failed and preclude clean quality acceptance. `tools/release_duration/finalize.py` correctly rejects such runs; do not bypass it.

The post-duration preservation helper is `E/final_preservation_after_duration.py`; it requires stopped owned processes and still-matching frozen inputs. It already completed for run9 **before** the timestamp edit. Do not rerun it against the changed source and mistake the expected fingerprint mismatch for a product regression, or repeat the expensive 124 GB material scan at every small checkpoint.

No background inference or new duration should be left running at this handoff.

## Final documentation review

The final review checked existing files and receipts only. All 56 literal path references in the original handoff existed; the installed symlink, recorded duration/review totals and timestamp source/patch/red-green log hashes matched. Added the preferred-tool hint gap, explicit model-independent loop guidance, exact P14 requirement location, memory-method limits and current no-test/no-restart instruction. No product source was changed in this review. `E/handoff-verification-20260913.json` binds this revised document; `E/handoff-complete.json` remains the historical receipt for its original revision.
