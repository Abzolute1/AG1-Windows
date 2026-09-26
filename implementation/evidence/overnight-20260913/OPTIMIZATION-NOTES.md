# Evidence-led investigation notes

These are observations and hypotheses, not claims that every listed item needs a code change. Candidate source remains frozen during each duration attempt.

Current status: the user stopped run9 and deferred further work. Earlier dated diagnosis/pending statements below are historical; the current source/install/validation split is in `../../HANDOFF-OVERNIGHT-20260913.md`. The final handoff review changed documentation only and ran no tests or inference.

## Confirmed fix

Host::dispatch rejected a completed session-resume command receipt under its original scope because an early exact-scope guard shadowed the historical-receipt exception. Existing client_receipt_contract reproduced the failure; the narrow receipt exemption fixes it while original-owner, workspace and mutation checks remain enforced. Receipt and independent conversation contracts passed.

## Current tool surface

product/crates/ag1-core/src/tools/surface.rs already hides read/grep/find/ls behind inspect, taskState/runtimeObservation behind work, artifact behind evidence, and legacy visual tools behind ui in new default selections. Explicit/restored selections remain exact. Advanced operations remain available through capability. Removing canonical tools would break restored selections and integrations; measure current default routing before considering more removal.

The Phase 14 repair driver explicitly selects eight legacy tools. Its results establish those paths, not the preferred surface. A separate sequential live preferred-surface exercise was completed later; its overlap and observation findings are recorded below. Eight-hour default-surface coverage is not claimed.

## First real task (overnight-4, task 0)

The model recovered the current historical contract, rejected obsolete behavior, changed one source line and passed the unchanged verifier. Final context: 29,712 estimated tokens, 48,000 target, workingSetReused=true, 266 unchanged history-prefix messages. Native final answer matched actual tools and external diff.

Avoidable work: used shell find for a workspace whose target was explicit; then read 16 KiB of artifact telemetry after search already returned the exact sufficient current contract. The legacy cursor read honors maxBytes overrides even for exact context cursors. This is compatible behavior, not grounds to silently change caller semantics. Preferred evidence retrieval should be compared for call count, payload bytes and recall correctness.

## Source-level performance hypotheses

Same-model conversation creation shares the local runtime, scheduler and HTTP service. RuntimeBundle.ensure checks cached material identity and health; it starts only when active model differs or an owned exit is observed. Live WebKit navigation now supports the tested same-model cases: run8 completed six new-chat/return cycles with the exact same model PID/start identity and zero reloads. Other transitions remain separately scoped. MaterialVerifier caches hashes by device/inode/size/mtime/ctime; do not reintroduce whole-model hashing on every chat.

WorkingSet keeps fixed-size raw prefix digests and an Arc of compiled history, but extend hashes full admitted raw prefixes and clones compiled JSON before each continuation. prepare_context also clones branch/raw message values. Potential cumulative CPU/allocation cost with growing journals; needs measured evidence and a revision-aware invalidation design before modification. Existing branch/source freshness witnesses must remain authoritative.

Measurement environment: shared desktop GPU (Discord observed at ~1,076 MiB before loading). Global GPU samples include unrelated apps. The test client fsyncs every protocol record and polls durable receipts; this is diagnostic overhead, so wall-clock end-to-end timings are not pure model or UI performance. Host allocator samples exclude model/WebKit heaps; owned process tree PSS includes them.

## Confirmed context transport defect and correction

Task 1's verifier passed, but Qwen interpreted the appended internal working-state user message as a fresh instruction and executed historical-run --phase 1. Exact compiled request and model output retained. Working-state metadata now joins the latest toolResult only at the request tail, preserving the tool-call ID, result contents, raw journal, stored objective, and all earlier message order. It no longer manufactures a user turn after tool execution. Initial-user/custom message behavior and generic SDK conversion stay intact. Core prefix tests (2), native wire test, todo/observation context tests, and all 9 platform working-set contracts passed. Subsequent live runs exercised the corrected transport without the historical command replay; other retained model-quality failures are separate.

## Broader tests: investigate after the frozen run

Core runtime suite: 42 pass / 2 failures. Existing todo query output and observation provenance labels intentionally differ from old exact SDK-oracle text; the changed transport is not called by these tests. Preserve current semantics.

Platform session suite initially 239 pass / 20 fail. Three transport-role assertions were updated for the intentional correction and all three focused cases passed. Nine working-set contracts passed separately. Remaining failures are retained in context-platform.log; do not call the full suite green. Some old event-count/phase expectations and custom executable fixtures appear stale.

A relevant source finding explains several repo/source failures: Git containment now calls git_read::metadata before invoking Git. A plain folder without .git returns DomainError::NotFound, whereas RepoTools::git_state and SourceTools expect the normal non-repository result to continue source orientation. Both fixture families intentionally cover ordinary non-Git folders. Investigate and fix this ordinary-folder behavior after the frozen workload; do not weaken contained Git or permit parent repository escape/custom executable execution. Optional .git/info directory assumptions also deserve a focused valid-repository case.

Desktop source hypothesis: NativeConversation retains all loaded messages and cached Turn objects and renders every loaded turn. Long chats can therefore accumulate DOM and React work even though the native projection is bounded. Need a separate native/UI large-history measurement before choosing virtualization; ordinary chat switching unmounts by scope, so this is not evidence of cross-chat leakage.

## Follow-up diagnosis while overnight-6 runs

The ordinary non-repository handling fix is now installed; session suite improved to 254 passed / 5 failed. In a source-unchanged focused diagnostic, the extension recovery fixture failed again. Captured native artifact bytes prove stdout/stderr merge order varies: configured prefix followed by failure (SHA c406dfe359e8032a555fde9b28eaa4cb175148d76cec6f18d66be2f454cb037b) versus failure followed by prefix (SHA 87826a8f6116ae7b7b35b211df4dd7fcb5bd4999af6e53d3d4364f97bbf6703f). The three native results therefore are not identical, and the conservative complete-output recovery guard correctly does not collapse them. Extension display is not the cause. Preserve raw byte semantics; make this fixture write prefix and failure to the same stream. Evidence: extension-recovery-diagnostic/test.log and captured-blobs.

Follow-up test changes were prepared in /tmp/ag1-overnight-followup with source base digests in followup-base.json, then selectively merged after run7 preservation. The validated candidate is now installed and frozen for run9. No live model inference or GitHub mutation is used by this fixture diagnosis.

## Valid Git metadata edge case confirmed

A disposable git-init repository remains accepted by system Git after removing optional .git/info and .git/objects/info, but AG1 repo status returns Tool execution failed: NotFound. The isolated test and optional-git-info-diagnostic.log retain reproduction. Merely skipping the mandatory pins is insufficient: bubblewrap cannot create missing overlay mount points below a read-only Git root (confirmed in an independent disposable namespace). Do not create those directories in user repositories from a read-only tool, bypass namespace containment, or silently label valid repositories non-Git. A correct namespace layout needs a separately measured design; this edge case is pending rather than fixed.

## Renderer long-chat measurement

Real NativeConversation with synthetic Markdown turns in headless Chromium, GPU disabled, isolated source copy: development-build 2,000-turn mount 2.54–2.90 seconds, append 43–58 ms, 62,032 descendant elements and roughly 69–75 MiB JS heap after GC. 50 turns append 4.8–5.5 ms. No browser exceptions. This demonstrates scaling in this synthetic view, not native WebKit latency or a leak. The production follow-up measured 1.48–1.75-second mounts and 9–12 ms appends, with about36MiB JS heap; still synthetic Chromium, not native WebKit. Evidence conversation-performance.json/mjs/png.

Production renderer comparison: the same 2,000-turn synthetic fixture mounts in 1.48–1.75 seconds and appends in 9.4–12.1 ms, retaining ~36 MiB JS heap after GC and the same 62,032 elements. Development timings must not be presented as installed-app performance. The ordinary native view initially supplies bounded history, so the full-mount fixture is a stress case rather than normal chat-open latency. Virtualization remains a measured scaling opportunity, but it must preserve per-turn expansion/hydration state, scroll anchoring, browser search/accessibility and streaming. No speculative rewrite was shipped. Production evidence: conversation-production.json/mjs/png and conversation-production-build.log.

## Redundant successful verifier calls under pressure (overnight-6 trial 29)

Source/recall/scoping checks and final claims passed, but Qwen ran node verify-batch.mjs four times despite success each time. The actual thinking repeatedly says verification is still needed, while attending to stale inspection indexes. Final estimated context was 47,673/48,000 tokens with three old thinking messages stripped in the last preparation. Exact compiled requests are retained in repeated-batch-requests; all original thinking and tool results remain in activities/000041-task/records.json.

A representative last tool result contains 22 bytes of verifier success followed by ~4.4KB request-local state, including the latest instruction again and twelve inspection indexes from six historical repairs. The footer says stale/missing/unverified indexes require fresh inspection. Staleness after a successful edit is expected and does not itself establish that a later verifier failed. Hypothesis: repeated task instructions and stale-index guidance dominate the small completed result under pressure. This is not a proven causal attribution. Candidate experiments after the frozen run: narrow index guidance to evidence actually being relied on; compare state-before-result ordering while preserving the tool role/identity; compare preferred work/evidence tool paths in fresh state. Do not equate successful arbitrary shell commands or silently suppress them: they can have real effects. The failure guard correctly counts actual failures only.

Owned model log timing summary runtime-timing-interim.json shows strong prefix reuse but occasional expensive reductions/cold admissions. Global shared desktop and auxiliary compilation/browser work affect speed; timing extrema are not isolated backend benchmarks. Preserved model settings remain exact.

## Additional live preferred-tool findings

- The default live catalog returned31 tools,13advanced; model final miscounted27/10. This is an answer-quality failure despite successful API calls, not a catalog-size measurement error.
- Evidence retrieval recovered exact264945-byte terminal output by query under the bounded result envelope. Explicit retain/release and advanced lease inspection worked; lease list endedempty.
- `work observe` silently accepted and dropped `kind=DECISION`, producing OBSERVATION while the model claimed DECISION. Native regression reproduced it; preferred schema/router now rejects mixed fields, and guidance directs decisions to `record`. Valid calls and recovery passed. A real old-journal reopen is being checked before duration.
- Shared task-state rendering still names `taskState` in its update hint even when the preferred selected tool is `work`. This is a wording/route-discovery inconsistency; no attempted unknown-tool execution occurred in this probe. Consider contextual tool naming without changing historical custody.
- Three self-reported observations were generated for one evidence/lease diagnostic; a successful task also repeated `work reset`. The preferred interface reduces separate state tools but does not itself prevent redundant model bookkeeping. Avoid making observation claims into independent verification.
- Original retry-header verifier misses invalid-prefix inputs. Live regex repair accepted bad4 as4, unlike retained initialSource's base-10 prefix parse. Counterexamples and failed manual review retained; do not treat original verifier success as complete semantic correctness.
- Strict cutover audit requires an empty global GPU window even for read-only acceptance. It is unsuitable as a shared-desktop investigation conclusion while Discord owns GPU memory. Formal acceptance remains strict; investigation final checks now explicitly use preserved-byte hashes and authority inspection without claiming the exclusive-GPU gate.

## CLI transport — measured and installed

Actual investigation CLI prefix:3578clientframes/272.19s,102.5MBclientframe records. This includes driver polling and repeated full views; it is not native desktopIPC volume. Source CLI ticks every16ms, serializes ClientView for equality, then clones and encodes changed views again. Isolated prototype encodes WireMessage once, compares exact bytes and writes the same bounded buffer, preserving byte-level deduplication and protocol limits. Alternating microbenchmark on29,241-byte realview: changed-frame encoding workload55.6%lower median elapsed; unchangedcase4.1%lower (small). This does not reduce traffic or establish end-to-endappspeedup. Actual JSONL framing/acknowledgement/EOF-drain testpassed. A broader future event/delta design must preserve scope/revision, missed updates, approval state and EOF lifecycle; not attempted in the frozenrun.

Two existing CLI fixture initializers lacked prompt_append/tool fields; they now use None to preserve their original intent. The selective merge and later fixture reconciliation are retained separately with source hashes.

Timed run repeated an erroneous final batch guard summary without the negation at taskindices4and10, whileactualsourceandverifierwerecorrect. Explicitfailedsemanticreviews retained. This shows sourceverification cannot validate natural-language summaries or prevent propagation of their mistakes. Another small tool-ergonomics option is returning explicit catalog totals/advanced totals so models need not count long metadata arrays; this has not been implemented or measured.

## Bounded preview projection — measured and installed

The native client formatted every historical message before retaining its recent 8,000-byte/100-message preview. A captured 787-record live branch took 1.012 ms per projection; counting message entries and lazily formatting the recent tail took 0.0469 ms (95.36% less for this step). Repeated synthetic 3,148/12,592-record inputs preserved exact serialized previews and counts, with 97.08%/97.96% less projection time. Four alternating sample orders; shared workstation; no end-to-end or total CPU reduction claim. The message count still scans raw entry types, while journal custody/paging is unchanged. Four projection contracts and three actual client/CLI integration checks pass. Patch/base hashes, selective merge receipt and validation evidence are retained; the change is in the installed run8/run9 package.

## Artifact preview catalog — measured and installed

A read-only metadata snapshot exposed a full accessible-catalog scan and temporary sort on each client refresh (2,140 accessible artifacts, Python SQLite exact SQL median2.885ms; diagnostic only). Native cold/warm testing on2,048 immutable artifacts returned identical20-record previews at median3.239/0.0386ms (98.81% less per warm query, not whole-app CPU). The candidate holds one cache entry with64KiB raw-record admission limit, keyed by session/limit plus SQLite own-connection total_changes and external data_version. Existing storage readiness and active-session checks run on every call; changes to permissions, imports, sessions or artifacts invalidate the cache. No schema or persistent format changes. Focused invalidation regressions and the full storage suite pass; combined current-baseline core/client checks also pass. Merged after run7 preservation and included in the installed run8/run9 package.

### Persistent custody snapshot duplication — measured, compatible redesign deferred

At 06:38 UTC, full changing journal snapshots (617 MB), unreduced runtime inputs (373 MB), and original requests (367 MB) dominate approximately 1.46 GB of unique artifact payloads after roughly 100 minutes. This is native durable data, distinct from CLI test logs. See `native-state-artifact-growth-interim.json`. `ContextStore::project` serializes a full base64 exact-byte journal and full request on each compilation; session context separately retains unreduced and compiled request envelopes. Whole-file content addressing only removes identical snapshots, so appending a turn creates another almost-identical large blob. Context virtualization bounds model context but does not itself bound these custody copies.

A read-only sample of eight recent artifacts per dominant kind shows 77–86% zlib size savings with exact SHA-checked reconstruction (`custody-compression-probe.json`). This demonstrates opportunity, not production performance or an implemented storage format. Prioritize versioned lossless compression and/or immutable shared snapshot chunks, preserving logical original-byte digests and complete reconstruction. Existing public `blob_path`, streamed artifact search/hash validation, bounded byte windows, import/export/cutover recovery and garbage-collection roots currently assume raw bytes. A safe change needs those consumers updated together plus corruption, decompression-size limits, crash durability, backwards reading and archive compatibility coverage. Do not delete retained snapshots or silently replace raw blobs with compressed bytes.

### Context custody temporary allocation and discarded snapshot — measured and installed

The compiler's shadow transaction retained the entire raw/normalized journal and base64 JSON until return, and read another full snapshot after reduction only to inspect its revision. Scoped custody serialization now drops the temporary buffers before reduction; the final check uses the same import readiness and active-session revision guards directly. Exact original bytes are still durably acknowledged before any reduction. Eight native contracts pass, including conflicts, archive/import fences and custody recovery.

Actual native allocator probe: 1,024 entries with 4 KiB content each, reducer-entry Rust allocation overhead 15,249,359 → 248 bytes. The initial serialization peak remains 32,747,444 bytes. This specifically reduces overlap with reducer allocations; it does not prove reduced allocator reservation, fixed overnight heap growth, or lower model memory. See `context-custody-allocation-assessment.json`. The native snapshot still requires full exact custody once per request; compression/chunking remains a separate compatible storage design.

### Long-lived tool runner artifact timestamp — fixed in source, not installed

Run9 live tool artifacts from requests08:12:55.758Z and08:14:59.142Z both carry createdAt08:12:55.624Z. Different producer call IDs and different output artifacts establish this is not content deduplication. Source path: client/mod.rs caches FileRunner initialized with timestamp(); session/files.rs execute_one passes self.timestamp to ContextStore; storage/context.rs writes it as createdAt. Approval/repetition TTL checks separately use the live Clock, so this is not evidence of expired authorization being accepted. Exact refs in run9-artifact-timestamp-finding.json.

The source follow-up now uses the existing request/ToolJournal timestamp for native tool-transaction custody, preserving explicit direct-runner timestamps and recovered/background-job provenance. The two-turn regression failed before and passed after; the other ContextStore call sites were audited. Broader validation, packaging and installation remain deferred. Old artifact timestamps and metadata remain immutable and were not rewritten.
# Changing verifier guidance invalidates an early prompt prefix

Run9 trials 33–36 report zero cached tokens on their first generation and substantial reuse during subsequent tool rounds. The preserved compiled requests show the canonical verifier changing at system-prompt character 2357, while hundreds of conversation messages remain identical. Two matched backend timings processed 34,310 and 35,510 uncached prompt tokens in 96.10 and 99.47 seconds. See `run9-prompt-cache-finding.json` for hashes, actual usage, request comparisons and backend log prefix binding.

This is an optimization lead specific to the measured print workload, which alternates six canonical verifiers. It is consistent with early-prefix invalidation; it does not establish a model-weight reload or equivalent behavior in ordinary chat with stable verifier guidance. Moving frequently changing trusted turn addenda closer to the request tail could help, but must preserve instruction authority, provider-template compatibility and the native completion gate. No prompt-policy or runtime-tuning change is installed for this finding.

Handoff update: the timestamp follow-up now uses the current ToolJournal date for new tool transactions. Its native two-turn real-shell regression failed before and passed after. Existing metadata and direct-runner fallback remain unchanged. No broader post-change suite, package or installation was performed; those and all remaining optimization work are deferred by the user. See `timestamp-followup.json` and `../../HANDOFF-OVERNIGHT-20260913.md`.
