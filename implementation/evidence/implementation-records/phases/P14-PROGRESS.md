# GitHub completed and installed — 2026-09-11

> Historical evidence, archived 2026-09-13. Status, commands, permissions and paths below describe their original checkpoint. For current work start with [implementation status](../../../README.md). Original bytes and old paths are preserved in the [relocation manifest](../manifest.json) and recovery archive.

[Final GitHub acceptance](../github/P14-GITHUB-FINAL-20260911.md): installed `/opt/ag1/releases/8bf70ba10402450a82054ea118f2b0b1`. 220 frontend tests, 77 GitHub units, 44 related contracts and 12 native desktop audits pass; production startup and all 1,142 installed files/receipt verified. Protected runtime/model configuration is unchanged. Seven historical frozen comparison failures remain unchanged. This is GitHub-only completion, not a new full P14 release acceptance. **User instruction: stop; do not start another plan item.**

---

# P14 execution checkpoint

The user authorized P14 execution after the accepted P13 handoff. Entry manifests,
accepted source fingerprints, profile/supervisor hashes and current quiescent
native authority passed read-only verification. Accepted P13 evidence is retained.

New implementation is under `product/release` and `tools/release*.py`. Evidence,
including unsuccessful attempts, is under `implementation/evidence/P14` and private
`.local/p14-*` directories. See [the current report](../../P14/REPORT.md) and
[gate status](../../P14/completion.json).

The first three offline replay attempts passed their actual reference/native tests
but were excluded because harness development changed the whole-run fingerprints.
Their raw logs remain retained. The fourth run passed with stable inputs at that
checkpoint (826 native tests); subsequent quality-driver development was covered by the fifth, final gate.
That gate passed 13 installer/validator tests, three quality-driver tests, 18
frontend tests, 49 unchanged reference probes and all 826 native tests, then
built the actual candidate with stable source fingerprints. It exited nonzero
with status `incomplete`, preserving the recorded quality failure and all eleven
required report gates. All 373 final artifact references were verified read-only.
The executed gate manifest is retained verbatim; a documented evidence closeout
corrects the package comparison after the final frontend rebuild changed desktop
binary bytes. This does not change test results or acceptance. Final binary
performance and reproducible builds remain unproven.

The first production preservation audit refused external Discord GPU occupancy.
After the user explicitly authorized stopping Discord for testing, a fresh
executable/ancestry identity check identified the current parent. SIGTERM through
pidfd caused a graceful exit and cleared its GPU process. No force or runtime
ownership adoption was used. A fresh full audit passed in 583.901 seconds.
The strict live matrix passed all 36 checks, 48 runtime events and final
preservation verification in `.local/p14-live-canaries-2`; its first
driver attempt was excluded before generation because the wrapper confused the
artifact-file hash with the native canonical report digest. That wrapper is fixed,
and both attempts' private evidence remain retained.

The first three-run matched context comparison found a substantial warm-path Rust
improvement with identical selected evidence/output hashes. It does not establish
live model performance or end-to-end task quality. The user was informed of the
measured scope and requested continued legacy-versus-Rust comparison.

The installed empty-home shell's measured idle CPU fell from 0.711% to 0.166% of
one core after eliminating unnecessary home-screen polling. Both five-minute
measurements include descendant processes and retain raw observations; they are
not a complete matched packaged baseline. The frontend has 18 passing tests, the
release validators/installer have 13, and the new frozen-prompt quality example
has three passing tests plus a clean Clippy check.

`release_quality.py` connects the unchanged pinned context-intelligence prompts
and verifiers to both TS and Rust provider adapters for all three tuned models.
Both adapters run both fixture variants. This measures answer/evidence behavior
under equivalent assembled prompts; it does not establish full-session harness
superiority. Its live execution completed in `.local/p14-quality/coherence-live-1` after
successful strict canary admission: 36 requests, 48 settled runtime events and
successful final material verification. Final scores were TS 18/18 and Rust 17/18;
the third model tied at 6/6. The wrapper correctly returned a quality failure.

The first quality model tied at 6/6 for each adapter. The second scored TS 6/6,
Rust 5/6: the Rust-side answer omitted a final JSON brace despite normal stop,
30 output tokens and equal requests. No stale/unsupported claim was observed.
The failed observation is retained; causality is unresolved without raw wire
chunks. P14 quality acceptance is blocked by this failure.

P14 remains incomplete until real duration/live/platform/performance/provenance
reports pass. Keep all originals, source pins, private archives and reference
copies. L01–L08 remain mandatory; no profile write, backend rebuild, tuning,
substitution, unowned stop or stale rollback is authorized.

The matched private shell pair completed: mean PSS 274.43 MiB AG1 versus
478.40 MiB original Electron Dyad (42.64% lower), idle CPU 0.156% versus
2.409% of one core. Both five-minute samples had stable process trees and no
leaked owned children. These shell improvements do not establish better model
coherence. The user intends UI work after P14 acceptance; it has not started.

Next P14 work starts with diagnosing the retained malformed response using
matched requests and raw transport capture, without changing the original verifier,
model settings or recorded failed result. Full-session fixture adaptation and
both actual 8h/24h workloads remain required, followed by complete packaged
performance/platform/signing/provenance/coverage acceptance. The current partial
measurements do not close those gates. No UI phase or public release has started.

User direction during continued P14 work: legacy parity is a regression floor, not the final product goal. The intended harness should enable reliable long-horizon tasks on smaller configured models, specifically Qwen 3.6 35B-A3B. Evaluate future context architecture changes by task success, evidence/recall correctness, revision handling and recovery across long sessions. Market-leading sophistication is an aspiration, not an established comparison result.

Continued preparation checkpoint: the four alternating raw-wire diagnostic pairs scored TS 4/4 and Rust 3/4. The captured Rust failure contained repetition in the server response itself, with decoded text matching the raw bytes; this does not retroactively explain the original uncaptured missing brace. Quality acceptance remains failed. The HTTP/canary/example checks pass (11/5/3), and the duration validator now has 14 passing tests including rejection of idle padding and missing actual activity.

Fixed a Linux resource-loader error where a long literal system/append prompt was treated as an overlong filename. All 23 resource contracts pass, including literal Unicode/long input, file overrides and out-of-capability symlink refusal. Reusable adapters now extract original long-horizon functions and task expressions without altering their scenario or verifier logic. Six original TS checkpoints pass with scripted responses and real session reopening. These are machinery rehearsals, not live model results.

Review corrected an initially insufficient native rehearsal: a successful scripted edit did not prove resume/recall. Native correctly refused a journal whose header named the seed workspace. Attempt 11 explicitly rebases only the derived fixture journal header, preserves all subsequent history/artifact bytes, and verifies selected session plus the exact current contract in the tool response. That one native checkpoint now passes. See long-horizon-preparation-review.json for current evidence and pending work. No eight-hour or 24-hour clock has started; actual ownership integration, token calibration, native full-checkpoint/fault/window rehearsals and sampled memory policy remain before duration admission.

The prompt fix also passes 29 registry and 240 session regressions (269 total). Prior accepted manifests and all new preparation references were verified read-only again. The bound supervisor matches its retained runtime material exactly and the original profile hash remains unchanged.

Full native rehearsal found and fixed a second Host defect: restoring a custom active-tool order filtered the host catalog without reconstructing the persisted order, causing strict resume to fail. The new actual Host reopen regression checks tool order and unchanged journal entries. With the fix, all six native scripted repairs pass, including five full Host shutdown/reopens, exact current contracts in actual tool results, original external verifiers, and permitted source-only diffs. All 64 original artifact blob hashes remain intact and all six final compilations stay within target. Both TS and native therefore pass the offline six-checkpoint rehearsal; this is not a live model or duration result. See full-session-rehearsal.json.

First live 2.5M-token preflight retained 2,497,717 exact externalized tokens. Legacy TS passed all six original repairs. Native passed three full checkpoint gates and the fourth source verifier/recall, then the driver incorrectly treated a paged context preview as complete statistics. A separate actual HTTP capture exposed a Host bug: selected thinking was never forwarded to provider options (TS low, native disabled). This run is unmatched and failed; see long-horizon-live-preflight-1.json, with all tested source bytes retained. Runtime settlement, preservation and original artifact integrity passed.

Fixed native selected-thinking mapping against 196 original streamSimple payloads across ten provider families, including the missing Mistral prompt_mode field. Actual HTTP tests exercise low/off/high and Host reopen. Historical command receipt lookup now allows exact completed records from retired session/host scopes while rejecting stale execution, foreign workspaces and forged/unknown receipts. All 831 native regressions pass. Full native context statistics replace bounded presentation previews in the repair driver; both TS and native now explicitly assert the selected reasoning controls and retain request bodies. Duration, renderer and archive/crash rehearsals remain in progress; no 8h/24h clock has started.

The latest duration controller rehearsal passed all six original repair verifiers and twelve actual controller activities through CLI/WebKit/native archive APIs, with full protocol slices and source/binary bindings. Four allocator-analysis regressions reject restart-hidden growth and missing observations. See duration-controller-rehearsal.json. The second matched live preflight has passed TS 6/6; native admission is ongoing. No 8h/24h clock has started.

Corrected matched live preflight 2 passed all six original repair gates in both harnesses, with matched thinking enabled and final preservation/settlement passed. All 64 artifacts and 2,497,717 exact seed tokens remain intact. The five warm tasks totaled 353.09 s in TS and 305.75 s in Rust (13.41% less time in this single pair); final source/verifier claims were supported, but failed TS taskState calls and native task-state echoes show no established coherence improvement. Exact sources/binaries and raw traces are retained in long-horizon-live-preflight-2.json.

The duration controller is now under tools/release_duration. Nineteen release/memory tests pass; the rebuilt Linux archive is 46b08abc0344ca30303675737ffd579b0ed43d77518c52399bea78e9c41446f7. The updated offline differential passed 50 reference probes plus 831 native tests. A real packaged recovery pilot is running at .local/p14-long-horizon/pilot-live-1, with minute allocator/PSS measurements. No 8h/24h clock has started.

Before freezing the duration candidate, finish the Rust standard-library notice closure (official local compiler distribution notices are staged privately at .local/p14-provenance-2/rust-standard-library-1.97.1) and make the final gate validate the exact measured candidate rather than automatically replacing it through another potentially nondeterministic frontend/native rebuild. Do not change the running pilot's frozen source inputs; retain its result first.

The first real packaged recovery pilot found a blocker: the first repair passed, but after an authenticated SIGKILL of the owned model the next task failed before provider output with runtime exited. CLI, supervisor and model all settled; CLI returned its failed-task status. The failed pilot and exact source bytes are retained in live-recovery-pilot-1.json. Four new tests reproduced the defect, including a real supervised fixture process. The runtime now reaps the verified exited owner before a fresh normal start, retaining custody on unresolved cleanup and refusing external endpoint adoption. All 21 targeted lifecycle/custody/recovery tests pass. See model-crash-recovery-regressions.json.

The final package also now retains the installed Rust standard-library notices. Candidate selection freshly verifies the exact previously measured archive and its bound build logs instead of silently replacing it through another build; 23 package/duration validator tests pass. All prior accepted phase manifests were reverified read-only and remain valid. Matched live preflight 3, a rebuilt package and the full updated differential are running. The real recovery pilot must be repeated on this code, followed by the actual 8h/24h workloads; neither duration clock has started.

Current candidate after the model-exit fix: archive SHA-256 621b9a85799d2d135fd8b5c740d8c3642c6223ae6fa13dc8de15890e6777a2e9; all 50 unchanged reference probes and 835 native tests pass. The installed shell crash/relaunch check passes on a private display, and all four interruption boundaries pass against these exact archive bytes in .local/p14-install-review-2. Actual provenance now passes its P14 validator in reports/provenance.json: 978 package files, 1,183 separate extension files, 1,239 renderer closure files and four native binaries reviewed. Full P14 remains incomplete. Matched live preflight 3 is running; TS repairs/recall pass 6/6, with one retained inaccurate final defect-description claim (route trimming was already present). New native tasks and the real recovery pilot remain before any duration clock.

Live preflight 3 is complete: six correct repairs and exact-current-contract recalls in both harnesses, 32 native requests, matched low thinking, original artifacts/pins and final settlement/preservation passed. Its 4,693 raw/source/binary references are in long-horizon-live-preflight-3.json. Five warm tasks measured TS 325.749 s and native 284.240 s (12.74% less), with no broad coherence-superiority claim. Additional review found the legacy child-tool PATH uses Node 24.14.0 while native tools/verifier use Node 26.7.0. long-horizon-toolchain-conditions.json explicitly excludes attributing the whole observed reduction solely to Rust; subsequent repository drivers should align tool PATH while keeping the pinned SDK runtime.

Real packaged pilot 2 is running at .local/p14-long-horizon/pilot-live-2. Initial admission and first repair passed; its authenticated owned model was killed and a replacement has started through ordinary native preparation. The next repair must still pass before recovery is accepted. A task-scoped systemd sleep inhibitor is active (screen lock allowed); release it by creating .local/p14-duration-preparation/release-sleep-inhibitor when all authorized long work ends. It also has a 48-hour maximum. Neither duration clock has started.

Private speed-probe preparation at .local/p14-speed-preparation preserves the original copy/reasoning assembly and verifiers. Four scripted HTTP/socket fixtures pass through TS fetch and native ProviderHttp, with exact request equality and unavailable speculative metrics explicitly recorded. The private owner uses the same global custody design; its secret/model/action contract passes. Its dependency lock has no drift from product/Cargo.lock. No speed live model has run; run.py must only launch when the endpoint is free. Private coverage drafts in .local/p14-coverage-1 map all 127 requirements, 62 with existing named current native tests plus 10 provider capabilities tied to 31 specific fresh native passes; child mapping and full release I29 performance remain pending.

Live pilot 2 proved ordinary model restart after an authenticated kill and exact completed-command recovery after host death. It then failed on a driver-selected branch target outside the fork history; native correctly refused it. duration-branch-driver-regression.json retains the actual native CLI red/green fixture with visible assistant tool-call commentary: corrected post-fork completed-checkpoint selection passes six repairs and all non-model fault scenarios, plus 23 release unit tests. No product code changed after matched live preflight 3. Candidate 9bd2df589a486fac9d3b04994a86002144ae02f41504eb23e9733a65f8f7166c includes this driver correction. Real packaged pilot 3 began at 2026-09-08 05:27 UTC. Both duration clocks remain pending successful recovery admission.

Candidate 9bd2df589a486fac9d3b04994a86002144ae02f41504eb23e9733a65f8f7166c now passes actual provenance validation (provenance-3), all four actual archive installer interruptions (install-review-3), and private-display shell crash/relaunch (screenshots byte-identical). The pilot remains live; its first two repairs and ordinary model-crash recovery pass. Private repository preparation now passes all four original scenarios in both full TS and native sessions, with exact original candidate verifier/evidence criteria and equal project-tool PATH. Instrumented native CLI outgoing bodies exactly match received scripted HTTP requests. repository-observer-rehearsal.json binds sources, binaries and all four matched fixture pairs. .local/p14-repository-preparation/run.py is prepared for 12 sequential live TS/Rust pairs, but has not launched and must wait for exclusive endpoint availability.

Pilot 3 has passed its first three source repairs, exact recall, model restart, host-crash receipt recovery, and corrected fork/branch. The real renderer reload/reconnect task is underway. First two answers have accurate final claims; third answer has a retained intermediate explanation error about the old clamp expression, although its fix and final verifier claims are correct. Reviews are in .local/p14-duration-preparation/pilot-3-claim-review. A guarded launcher (session 12319) at .local/p14-duration-preparation/launch_8h_after_pilot.py is armed to start .local/p14-long-horizon/8h-1 only after pilot 3 finishes all six repairs, every recovery scenario, cleanup and full preservation audit with status passed. It refuses failed pilots. Watch 8h-launch-starting.json and 8h-1.log; the actual duration clock is started.json inside the duration output. No duration has started yet at this checkpoint.

Startup observation calibration on the uninstrumented production archive measured a single complete first paint at 862.212 ms, using reviewed exact heading/navigation/project-opening pixels. The first attempt correctly did not match a dark screenshot in the isolated light-theme environment; both attempts are retained. This is method calibration under concurrent pilot load, not a p95 performance pass. Fourteen provider/persistence child families (66 cases) now have explicit actual current native test mappings in .local/p14-coverage-1/provider-storage-children.json; 30 child families remain. The aligned all-model long-horizon private driver is prepared at .local/p14-long-horizon-matrix-preparation/compare.py (--model ID), with six scripted TS checkpoints and the owner contract passing. Do not run new competing live comparisons while duration owns the endpoint.

Pilot 3 now passes all six original repair/recall gates and every real recovery scenario, including actual private WebKit reload/reconnection and corruption refusal/restoration. The final full production preservation audit is in progress. All six answers were reviewed; zero stale-authoritative claims, one incorrect intermediate clamp explanation, and one broad final batch-summary phrase remain explicitly recorded. The guarded eight-hour launcher is still waiting for the closing audit and passed pilot receipt.

Recovery pilot 3 completed: six original repairs plus actual model/host crash, fork/branch/resume, renderer detach/reconnect and corrupt-state recovery passed; final production preservation audit passed. All source/binary/raw/semantic-review artifacts retained in live-recovery-pilot-3.json. One intermediate explanation error and one final wording limitation are retained; no general coherence superiority is claimed. Actual 8h-1 started 2026-09-08 06:34:15 UTC (08:34:15 Oslo), earliest elapsed finish 14:34:15 UTC plus final audit/review. Candidate 9bd2df58 remains frozen.

During 8h-1: first three repairs and fork/branch/resume passed; actual per-task claim reviews retained, including intermediate reasoning/retrieval inefficiencies. All 44 child families (149 cases) now mapped to reviewed current native tests, preserving unsupported/reference dispositions; final parent/I29 release closeout remains pending. Private storage/retrieval measurement rehearsal passes exact pages, equal-size corruption rejection under a common integrity contract, 30 durable appends and reopen. TS original raw timings remain separate from explicit caller hashing/fsync; no formal performance pass from concurrent rehearsal. Production GUI external-XTest calibration now operates the exact installed archive on private authenticated Xvfb without an app test broker.

8h-1 task 4 route repair passed the unchanged original three-case verifier and actual renderer reload/reconnect, but semantic review found a real contract violation: empty/whitespace input returns empty rather than one leading slash. Independently confirmed from the retained acknowledged edit without modifying the live workload; failed claim-review 000003.json and 8h-1-route-semantic-failure.json retained. Do not accept 8h-1 quality or weaken the reviewer to erase this failure. Continuing the full elapsed workload to collect memory/recovery/stability evidence; later acceptance remains unresolved.

Additional installed Linux checks: CLI protocol + locked Vite install/run + actual HTTP contents + stale-session stop refusal + listener revocation + zero observed owned children passed in .local/p14-linux-platform-2. The first adapter attempt mistook synchronous completed acknowledgement for rejection; retained. Actual production desktop preview renders and responds to input, but Close preview incorrectly opens Quit AG1 and leaves the preview open. An isolated product copy at .local/p14-preview-fix adds a main-window label guard. Its actual private packaged reproduction passes preview close/reopen/project-stop closure and preserves main-window quit confirmation. linux-preview-close-regression.json retains 2,814 refs and the original failure. This copy is now immutable evidence; do not edit it. Canonical product still unchanged; adoption and formal package reruns remain pending. The first experimental archive was correctly rejected for a stale payload manifest; corrected private archive retained with explicitly documented experimental fingerprint serialization.

8h-1 completed 15 original repairs. Claim reviews 0–14 retained; routes at indices 3 and 9 fail semantic contract. Task 15 (000024-task) is stuck repeatedly echoing taskState JSON via bash after original verifier success. Read-only SQLite/immutable blob capture proves actual request lists eight tools without taskState while task guidance says to update it and retains historical phase-1 telemetry as objective. Snapshot contains 13 echo calls and zero runtime continuation/completion nudges. 8h-1-task-state-loop-diagnosis.json retains the evidence; this is a plausible context-guidance contributor, not yet a proven fix. Allow frozen 1,200-second task timeout to resolve; do not force a duration pass. No 24h started. All accepted P01–P13 manifests (149 P13 refs), profile and pinned supervisor reverified read-only at ~07:29 UTC.

8h-1 ended failed after 4,108.690 seconds (68.48 minutes), 15 completed original repairs and 69 memory samples. Frozen 1,200-second deadline stopped task 16. EOF only drains, so its exact retained owned model was gracefully terminated for failed-run cleanup. All 11 sampled identities exited and the full final production preservation audit passed. duration-8h-attempt-1.json binds 8,156 artifacts; neither duration passes. One short lifetime has 15 post-warmup samples fitting 1.33 MB/hour, useful diagnostic data but not eight-hour memory acceptance.

Adopted the independently tested main-window preview-close guard. Added actual-request-tool-aware task-state guidance: disabled taskState remains read-only evidence and no longer instructs unavailable bookkeeping/show calls. Actual HTTP red/green checks include reopen and dynamic tool selection; two core freshness/clipping checks and all 41 unchanged runtime contracts pass. Timeout driver now sends exact-operation cancellation before closing input and preserves the explicit unresolved custody fence. Interrupted HTTP deliberately does not prove remote generation settlement; temporary cancellation tracing was reverted. context-guidance-regressions.json retains initial mistaken test assumptions and corrected evidence. Full 50-reference/838-native differential and frontend build pass. Acceptance target was checked for absence of temporary trace markers.

Current frozen inputs are 6b76b99ed74d9bdeb312765d0a13c2222a9e918ba1739185c22d7946eb5744ec, retained in .local/p14-guidance-source-checkpoint (2,896 files); six current support binaries are in .local/p14-guidance-binaries-checkpoint. New canonical packaging is running (session 46091). Next: current-package validation and the prepared aligned Node live comparator, .local/p14-long-horizon-matrix-preparation/compare.py .local/p14-long-horizon/live-4 --model Qwen3.6-35B-A3B-UD-Q4_K_XL. No live generation is active. Corrected guidance still needs actual model validation; do not claim resolved coherence or route semantics from unit tests. 24h remains unstarted. Sleep inhibitor session 43693 remains active.

Current canonical package b1145db9df98f71f650bd5ef68581141021434bea287a0692ad39329f5105734 (inputs 6b76b99e) passes actual installed Linux CLI project/protocol/listener tests, four installer interruption boundaries, and production GUI preview controls/main close confirmation. GUI attempt 2 retained a test-driver smaps_rollup ProcessLookupError during process exit; cleanup had zero leaks. Fresh GUI attempt 3 completed with zero leaks and zero model requests. linux-installed-current.json retains 10,693 current-package evidence files. Current shell crash/relaunch smoke passed with zero leaks; concurrent live comparison excludes its 0.168862% idle CPU from formal performance acceptance. Child mappings refreshed against all 838 native regressions in child-coverage-current.json (44 families/149 cases).

Aligned project-toolchain live preflight 4 is running under sole runtime ownership (session 11440), TS then Rust, 2,497,717 externalized tokens. TS actual traces include an unavailable taskState command invoked through Bash; current native guidance is intended to address this but remains unproved until actual answers are reviewed. No new duration has started; failed 8h-1 remains excluded. Current provenance-4 publisher is running after current archive closure/dependency/visual review.

Current provenance-4 passed and its 37 artifact references verify against exact input digest 6b76b99e and package b1145db9. All 10,693 installed Linux references, 1,049 rebuilt repository/quality-helper references, and 16 refreshed coverage references also verify. Rebuilt follow-up binaries are isolated in .local/p14-followup-target; .local/p14-repository-current-preparation/fixture_observer.py passed all four original matched scripted cases in .local/p14-repository/fixture-4. That current preparation directory and followup-build files are now publicly bound: do not edit them in place. Prepared actual live commands remain queued, not started.

Live preflight 4 TS completed all six original verifiers/recall checks, 31 requests, and all six supplemental semantic reconstructions. Actual final claims are supported; all six tasks nevertheless attempt an unavailable taskState shell command, receive command-not-found, then recognize it and finish. One retry-header intermediate phrase about Number parsing is overbroad, retained in .local/p14-duration-preparation/reviews-live-4/ts-6.json. No general coherence win inferred. TS authenticated runtime settlement and preservation passed; current native side is in admission verification, PID 530805 observed at 08:37 UTC with approximately 246 GB read. Native model task output remains pending.

Current runner discovery confirms no Git remote, gh/native Windows/macOS runner tooling, or project signing credential variable names. Generic gpg/ssh existence does not prove release identity. platform-prerequisites-current.json records these still-pending gates. Formal first-paint (20 samples) and display (three sessions/60 exact text frames) series drivers are prepared privately under .local/p14-performance-preparation and have not run; run only when live/build/audit load is idle. No duration pass, no 24h start.

Matched live preflight 4 fully passed and settled with preserved runtime/pins/source. Both adapters pass all six original verifiers and six supplemental semantic reconstructions; current native uses 24 provider requests versus TS 31 and makes zero unavailable taskState shell calls versus TS six. Five warm tasks sum to TS 303.326686 s vs native 235.524036 s, 22.353% lower in this one fixed-order pair. First native task includes lazy preparation and is not a matched cold latency comparison. Native route reasoning contains an explicitly self-corrected intermediate slash-collapse error. All twelve actual reviews and raw task evidence are retained in context-guidance-live-observations.json.

Actual 8h-2 began at approximately 2026-09-08T08:47:48.044146+00:00 (monotonic 1178197050432589), earliest eight-hour elapsed finish 2026-09-08T16:47:48.044146+00:00 plus closing audit/review. Guard session 89445 now owns the duration driver. Current package b1145db9/input 6b76b99e remains frozen. Admission uses the complete reviewed current matched preflight and installed-package checks; prior successful real pilot 3 retains recovery-flow evidence, while 8h-2 itself must execute every required fault on this exact candidate. No extra pilot is required by the duration admission contract. Review tasks as they finish. No competing live generation, benchmark, build or package mutation during the duration. No 24h run has started and no duration pass is claimed.

8h-2 first three original repairs and branch/fork/resume passed; actual source/final-claim and supplemental semantic reviews 0–2 are retained. No unavailable task-state bookkeeping calls. Backoff did unnecessary extra artifact paging; final expression and verifier claims are accurate. Real renderer reload/reconnect task 4 is underway. Separate current installed CLI paused-reader fixture passed 45 seconds of real OS pipe backpressure while its scripted provider completed. Exact 1,048,601-byte durable response and completed receipt survived resume; zero leaks. Observed conservative pending transport upper bound 82,940 bytes stayed below 131,073-byte pipe+wire-frame budget. paused-installed-cli.json retains this actual Linux CLI scope; desktop IPC pause remains a separate check.

8h-2 task index 3 (000008-task) passed actual renderer reload/reconnect and the original three-case route verifier, but FAILED manual semantic review: empty/whitespace input stays empty, violating exactly one leading slash. The same seven supplemental cases all pass against the original fixture initialSource, confirming this is an actual contract miss. Final answer also overstates missing operations before acknowledging trim was already present. claim-reviews/000003.json and 8h-2-route-semantic-failure.json retain the failure. Current run continues for actual elapsed memory/recovery/stability observations; it cannot receive quality acceptance. Guidance has prevented unavailable bookkeeping calls so far but has not solved general source-verification sufficiency. Do not weaken finalization or silently discard this failure.

Actual current production WebKit renderer pause passed in gui-paused-2: identity-checked pidfd SIGSTOP for 45.338 seconds, unchanged frozen pixels, native command completed durably while renderer remained stopped, and full 520,343-byte text survived actual installed CLI reopen. After SIGCONT the UI repainted to idle and bounded journal excerpt. Peak observed owned Unix IPC backlog 227,960 bytes stayed under the predeclared 8 MiB fixture budget; this is conservative socket accounting, not renderer-only allocator bytes. Zero leaks. First attempt had a test event-count assertion (one request plus 128 sent chunks), retained unchanged; corrected driver counts sent chunks explicitly. paused-production-renderer.json binds actual screenshots, syscall/process/socket observations, durable records and sources. Concurrent endurance load excludes latency/CPU performance claims from this test.

Current native longer-run task 20 took 331.0 s with four large uncached prefills (27,306 / 26,623 / 25,311 / 33,630 reported input tokens). Actual core prepare/admit/terminal/prepare fixture confirms native recaptures unreduced history, unlike pinned TS AgentSession retaining its compiler projection. Both actual compilers match on identical fixture stats: raw next input 162,663 estimated tokens and repeated reductions versus retained 15,789 with zero further reductions. context-working-set-retention-diagnosis.json binds actual core/TS runs, source and complete-line live log prefix. This is a concrete retention difference and plausible live cache-churn contributor, not measured causal attribution or a semantic fix. Candidate remains frozen for 8h-2. New cache-observer helper is isolated in .local/p14-context-cache-target and publicly bound; do not overwrite it.

At 10:12 UTC the current Linux platform report validates all nine installed/archive scenarios and 15,600 references. Actual 8h-2 corruption recovery completed: refusal retains damaged bytes; restored archive and installed CLI recover all 64 original artifacts. Task reviews 0–23 complete, route 21 repeats the empty-input error and is retained failed. Source-isolated working-set fix now passes seven actual HTTP/session contracts; full workspace tests running in its own target. It retains canonical history only after admission, uses fixed-size raw-prefix fingerprints, revalidates sources/leases, and invalidates on owner/branch/configuration change. Production and frozen 8h-2 inputs unchanged. Startup series fixture 2 passed two actual installed CLI/scripted-provider samples; no live startup or performance acceptance inferred.

### 2026-09-08 10:48 UTC — second endurance attempt closed; correction adopted

8h-2 stopped after 6,322.56 seconds, 30 repairs and 106 samples. All tasks reviewed;
five route boundary failures and one premature intermediate edit claim disqualify
quality. Required model/host crash schedules were not reached. All owned identities
settled, three CLI drains returned zero, and final full preservation audit passed.
Raw old KeyboardInterrupt status defect is retained with an explicit failure wrapper.

Adopted the isolated context-working-set correction and generic source-verification
guidance after 846 distinct native checks, retaining the generated-app fixture setup
failure and exact successful rerun. Six review/interrupt driver controls pass; all
Python release controls pass. Fresh canonical 50-reference/full-native differential
and separate visual/support build running. New candidate requires fresh package and
reviewed live comparison; no accepted eight-hour or 24-hour result exists.

### 2026-09-08 11:10 UTC — current package validated; preview polling race found

Working-set candidate 641e74a0 passes fresh 50-reference/846-native differential,
29 Python controls and frontend build/tests. Linux archive 2a61f892 built; exact
source and eleven binaries retained. Current installed CLI, four interrupted-install
cases, shell crash/relaunch and formal provenance pass. Full production preservation
audit passed in 649.79 seconds; strict canaries running with frozen current inputs.

Actual production GUI first preview-close was refused with QueueFull; a later explicit
retry exited the child and the external sampler hit ProcessLookupError reading smaps.
All owned processes settled; partial attempt is failed, not promoted. Isolated native
preview actions now await the existing Host controls gate before authority/scope and
project access, so the background poll cannot reject an otherwise valid action. Two
regressions fail the immediate old path and pass the correction; cancellation/stale
scope cannot run callbacks. Desktop compiles; full isolated suite running. Sampler
ESRCH correction is also isolated. Matched live launcher held before spawning any
model child; no accepted eight-hour or 24-hour result yet.

### 2026-09-08 11:44 UTC — preview correction installed; matched live comparison launched

Current source caac6ad3/archive 4967cc57 passes 50-reference/848-native differential,
29 Python release controls and frontend build. Exact source and seventeen binaries
are frozen. All 36 strict canaries passed on preceding 641e74a0; only explicit preview
control, regression tests and process sampler changed after them. Actual current
installed CLI/project, GUI preview close/reopen/error/stop/quit-cancel, and four update
interruption cases pass, zero owned leaks. GUI5's initial project reached its existing
five-minute deadline during inspection; restart and all raw observations are retained.
Current matched TS/Rust live-5 is launched with preserved 35B configuration. No fresh
coherence or endurance pass yet. Steam download/install may be active; formal speed
measurements held and overlapping timings excluded from fair speed claims.

### 2026-09-08 12:04 UTC — current Linux checks closed; legacy baseline independently reviewed

Current installed CLI, GUI preview and update cases pass; crash/relaunch visually
reviewed and complete with zero leaks. Current Linux provenance validates. Paused
CLI and renderer pass 45.21/45.69-second actual stalls, full response recovery and
zero leaks; raw references verified. Exact 848-test child mapping refreshed (44
families/149 cases), accepted P01–P13 manifests reverified read-only by mapper.

Live-5 legacy passes all six original verifiers but independent route empty/whitespace
checks fail. Batch source passes while final table inverts its return-empty predicate;
that explanation defect is failed review. Other four claim reviews pass, with backoff
wording imprecision and duplicated completion/read inefficiencies retained. No absent
taskState calls this TS run. Legacy runtime settlement and preservation passed;
current Rust side running. Final P14 quality acceptance remains pending.

Private native-duration launcher now requires every native source/claim/semantic
review passed and all original TS/Rust verifiers passed; all six TS reviews remain
mandatory with failures retained. It removes only the earlier private launcher's
additional requirement for a flawless legacy explanation/semantic baseline. This
admits independent native endurance measurement, not overall live/P14 acceptance;
canonical validators and original fixtures are unchanged. See live5-duration-admission-policy.json.

### User hold — eight-hour startup interrupted before task execution

User explicitly instructed not to start eight-hour work yet. Live-5 had fully closed
and armed launcher entered 8h-3 admission about 61 seconds before stop handling.
Exact launcher was frozen, exact duration driver interrupted, launcher terminated;
all recorded owned identities exited. No task or client protocol command occurred.
Closing the launch session prevented normal duration-result/drain receipts, retained
explicitly without a graceful-drain claim. No restart without user instruction.
Live-5 native source/semantic/final reviews pass 6/6; legacy semantic5/6 and claims4/6
remain failed baseline observations. Both final preservation/settlement checks pass.
GitHub private backup cd682ff was remote-verified; token transient only. Source digest
caac6ad3 remains unchanged. Local private evidence is outside that Git backup.

### Continued non-duration work and model feedback request

The user reaffirmed the eight-hour hold while authorizing other P14 work. The
original four-case repository matrix is running sequential TS/Rust pairs across
three models. The first TS export case implemented correct source and passed its
untouched verifier, but failed the original required-artifact-rehydration gate;
that distinction is retained in its independent review. No native result inferred.

Current Linux archive recovery passed on disposable state with the installed CLI:
specific corrupt-database refusal, exact conversation and completed-receipt restore,
damaged bytes retained, no inference on restore or production authority transfer.
Initial fixture lacked state/host.json and was refused before corruption; retained.
The current nine-case Linux report validates 11,997 artifact references. No duration,
model quality, formal performance or other OS/signing acceptance is inferred.

User requested human-style model feedback. Prepared a neutral, identical same-session
debrief after a separate fresh matched original task, with task/debrief evidence and
counts separated. Scripted TS/native rehearsal passed with zero local model calls.
Live feedback waits for exclusive runtime settlement; every material self-report
requires trace corroboration. Earlier model-as-inhabitant reports and their
contradicted complaints were read as methodology, not current product evidence.

User asked about disappearing diffs in Rust. Direct inspection of all 24 live-5
native provider requests verified all six small edit diffs unchanged in every
subsequent request through verification/final answers. See
live5-native-diff-retention.json. Deterministic large-result tests preserve exact
artifact bytes and executable paging; reliable live large-diff retrieval under
sustained pressure remains unproven. No eight-hour or 24-hour run has restarted.
# 2026-09-08 — second repository pair and corrected visibility measurement

The second 35B repository pair repaired both retry-delay production boundaries and
gave accurate final explanations in TS and Rust. Both pass the original verifier
and seven valid-domain supplemental checks. Four invalid-input probes fail equally;
the predeclared ambiguous-scope policy retains these as robustness observations.
Rust used 11 requests/16 generated task tools; TS used 8/13. No speed inference while
the user's Steam download is active at 40%.

The native private scoring adapter incorrectly fed immutable raw history to the
original resident-evidence predicate. Actual outgoing requests retained the full
exact failure diagnostic throughout. A separate offline adapter uses those compiled
tool messages for residency and keeps all original criteria and other metrics.
All four closed-side recalculations passed consistency checks: the first pair keeps
its original TS failure/Rust pass; the second becomes TS pass/Rust pass. Negative
controls still reject absent exact evidence. Original failed reports and all active
matrix inputs remain untouched. Published repository-second-pair-review.json and
repository-residency-adapter-correction.json bind the findings. No product changes.

Eight-hour and 24-hour workloads remain held pending explicit user direction.

# 2026-09-08 — user-requested stop and next-model handoff

All active comparison work and the waiting functional queue are stopped. Five
repository pairs are complete; m1-c1 TS finished with custody/preservation verified,
while native was interrupted during initial admission before any protocol command.
Its CLI drained with exit code 0 and no cleanup error. No matching workload
process remains. The final matrix audit was not reached. No further tests were
started for this handoff.

All four 35B repository source contracts and final behavior explanations pass in
both adapters. Evidence compliance is TS 3/4, Rust 4/4 after independently controlled
measurement corrections; Rust used more calls in this suite. The 27B export source
contract passes in both, but Rust falsely claimed non-string coercion was unchanged;
null/undefined reproduction contradicts that claim. Do not claim uniform coherence
improvement. Two original scorer blind spots and their corrections are documented
in separate immutable reports. Actual model debrief remains unexecuted.

An inherited shell persistence-hint scanner bug mistakes quoted JavaScript arrows
for redirects. The isolated draft commit 7ec917ba591850d616b7771324cb250c120b63d3
retains a conservative scanner and regression test: original code fails the new
case; the isolated tool-contract suite passes 24/24. The draft is not adopted into
the frozen candidate; further review is pending. A portable patch and both logs
are versioned under evidence/P14/drafts.

Read [P14-RESUME-HANDOFF.md](P14-RESUME-HANDOFF.md) for evidence, exact next steps,
remaining gates and preservation boundaries. All work is paused; eight-hour and
24-hour workloads remain specifically held until explicit user direction.

### 2026-09-08 — resumed from the explicit stop handoff

User resumed functional P14 work. Entry identity/dependency/stop-receipt checks passed;
production authority remains native/quiescent and no workload processes were present.
Fresh preservation audit precedes generation. Independent m1-c1 TS review passes
source/final claims and seven valid-domain checks, retaining four ambiguous probes.
One-pair continuation and explicit-input cursor scoring drivers prepared/checked.
Shell-hint draft review reproduced two further false positives; a second unadopted
portable draft passes an extracted scanner contract. Product candidate unchanged.
Duration remains held; performance readiness has been requested separately.

### 2026-09-08 — actual application-menu launch repaired

User reported SUPER+R AG1 icon did nothing. Reproduced old visual-test executable
exit 101 (fixture broker required). Installed exact verified candidate per-user,
retained old desktop entry and repointed AG1 to its stable launcher. Direct
Wayland package launch also failed (GDK Error 71); existing host X11 + disabled
WebKit DMABUF workaround produces a rendered real desktop window. All 977
installed files verified; menu entry validated/refreshed. Package/source unchanged.
This is fresh evidence of default-launch/platform limitations, not a phase pass.
Subsequent comparisons held for hands-on user testing after m1-c1 settles.

### 2026-09-08 — original interrupted retry pair closed

27B m1-c1 resumed native side passes source/final-claim review and all eleven
supplemental probes; TS passes seven valid-domain checks and retains four ambiguous
hardening failures. Rust used nine requests/fifteen generated tools, TS seven/eleven.
Native corrected its own invalid supplemental import command. Raw residency scorer
failure retained; actual outgoing evidence corrects to pass with unchanged criteria.
Native drained exit 0, cleanup null; stopped matrix inventory unchanged (1,780 files).
Next 27B stale-contract pair launched in continuation-2. User reaffirmed continuing
P14 and deferred project-creation/model-before-workspace UI complaints.

### 27B stale-contract pair reviewed

Both adapters pass source, final claims and four independent supplemental checks.
Identical production changes; native nine requests/fifteen actual tools versus TS
eight/eleven. Repeated searches are recorded as avoidable overhead. All 1,780 old
matrix files remain unchanged; native drained successfully. See
`evidence/P14/repository-27b-stale-pair-review.json`.

### All remaining work authorized except duration

The user authorized every remaining P14 activity except starting eight-hour or
24-hour workloads. These holds remain explicit. Steam is absent at the latest
inspection; formal performance will follow functional work in an observed quiet
window. The full 27B repository set is reviewed: both source/evidence 4/4, final
claims TS 4/4 and Rust 3/4. Native 33 requests/48 actual tools versus TS 30/40;
the export false claim remains failed. Flash export is running in continuation-4.

### Shell-hint draft review completed

Version 3 reproduces and fixes the remaining Bash dollar-quoted false target hint.
Exact retained Node commands no longer predict brace/min/max files. All 849 native
workspace tests pass in the isolated checkout; core Clippy and changed-file rustfmt
checks pass. Missing fixture/tool/debug-path setup failures remain retained.
The frozen 848-test candidate and all seventeen binary identities are unchanged.
See `evidence/P14/shell-hint-draft-v3-validation.json`.

### Startup admission measurement corrected before live series

Source inspection confirmed capture verification precedes CLI readiness. Prepared
version 2 timed only prompt-to-model-exec as admission. Version 3 includes the full
pre-model interval and retains its component clocks; synthetic ordering controls
and two actual installed CLI fixture samples pass. Live twenty-sample startup is
still pending. No product or tuned runtime changed.

### Existing Linux and provenance reports revalidated

Both exact-candidate reports under `evidence/P14/reports/` validate read-only,
including 12,034 artifact references. The older top-level Linux note incorrectly
left archive recovery pending; the later final Linux report already included it.
An additional scripted installed recovery check restored four records exactly,
retained corrupt bytes and reopened idle. Initial fixture preparation failures
remain retained. See `evidence/P14/existing-release-report-revalidation.json`.

### Three-model repository matrix completed and reviewed

All twelve pairs have correct source in both adapters. Original evidence criteria
pass TS 11/12 and Rust 12/12 after controlled visibility/cursor corrections.
Final claims are supported TS 10/12 and Rust 9/12: both have two contradicted
cases; native also retains one unsupported historical input assumption.
TS used 95 requests/120 generated tools; Rust 119/143 (19.2% more tools).
No uniform explanation or efficiency improvement is established. All 1,780 old
matrix files remain unchanged. See `evidence/P14/repository-three-model-matrix-review.json`.
The neutral 35B same-session feedback check is running in feedback-live-1,
followed by its full preservation audit. No 8h/24h workload was started.

### 2026-09-09 — improvement requirement and current execution correction

The user explicitly states that AG1 TypeScript is the floor, not the ceiling;
the Rust port must substantially improve on it. Parity and existing gate passes
alone do not establish this outcome. Judge improvements against retained matched
evidence for correctness, supported explanations, evidence retention, redundant
tool calls, responsiveness/resource use and reliable project/session handling.
Do not suppress regressions, weaken criteria or describe isolated wins as an
overall advantage. No numeric “massive improvement” threshold has been agreed.

Prioritize implementing and adopting validated fixes, then measuring the updated
candidate, while preserving the completed old-candidate matrix. Shell-hint v3 is
applied in `.local/p14-feedback-fixes-1` but remains unadopted and uninstalled;
workspace prompt composition and verifier-cache transfer remain investigations.

The feedback attempt referenced above has stopped: its TS owner timed out after
334.77 seconds, with runtime settlement and preservation verification recorded.
No TS task-result file was produced, and the wrapper failed reading that missing
file. Native feedback and the wrapper's final full audit were not reached. This
attempt supplies no task/debrief quality conclusion. No model/feedback owner was
running on inspection. The 8h/24h holds remain unchanged.

### First feedback-driven improvement batch adopted

Adopted fourteen source/test files after 853 native workspace tests, fifty unchanged
reference probes and core/platform library/binary Clippy passed. Full production
preservation audit passed before adoption. New source identity:
`38cde1d2ac32b3b8593f04ac8086324d1a8c590d455163c33af73a7dba1e1e05`.

Changes adopt the validated shell-hint correction, supply bounded historical
workspace grounding through refresh/reopen, carry a freshly verified material cache
within one held runtime lease, and keep evidence/claim guidance concise. Earlier
failed drafts and test assumptions are retained; no new live performance or quality
advantage is claimed. Old public reports and source/binary mappings are retained
under `.local/p14-public-before-feedback-fixes-1`. The package and observer helpers
are rebuilding. Installation and the prepared matched 27B stale-contract diagnostic
remain pending; both duration runs remain held. See
`evidence/P14/feedback-driven-improvements.json`.

The updated package built; all 29 release controls passed. The actual packaged GUI
opened the disposable workspace, sent the new grounding, rendered all twenty
scripted markers and drained its owned children. Installed all 977 verified files
into the existing per-user AG1 menu installation, retaining its previous release.
All four rebuilt repository adapter fixtures pass. The fresh matched 27B
stale-contract pair is running in `.local/p14-repository/improvements-1`.
See `evidence/P14/feedback-improvements-installed.json`.

### First updated matched diagnostic reviewed; preservation settled

The updated 27B stale-contract pair passes the original source/evidence criteria
and all four supplemental semantic checks on both sides. Rust used seven requests
and nine actual task tools versus TypeScript eight/ten, improving on historical
Rust nine/fifteen. Native final-claim review fails: its numeric-input verification
example was actually executed with a string. Retain this reporting regression
alongside the efficiency observation; no overall advantage is established.
The full final preservation audit passed, source/binary inputs and original pins
are unchanged, and production authority remains native epoch 2 and quiescent.
See `evidence/P14/feedback-27b-stale-pair-review.json`.

The larger-workspace/long-input reproduction was a single-response fixture running
into automatic compaction. Supplying the summary and main-generation responses
passes completion, full Unicode input retrieval and fork. All 27 CLI contract tests
pass with the supplemental regression; its replayable patch stays separately bound
to this measured candidate without changing main product inputs. See
`evidence/P14/feedback-long-input-fixture-review.json`. Prior failed observations
remain retained. The eight-hour and 24-hour holds remain unchanged.

### Second feedback-driven improvement batch adopted

Current source is `a411ce39c86dc2602d14c08f54b7a079ae78473742bdc1de10c00067767eb695`.
Artifact grep exposes total matching lines and match-list truncation without an
additional scan, while authenticated cursors and original fields retain their
contracts. Guidance explicitly preserves tested input types and marks unrun examples
untested. The larger-workspace/long-input compaction/fork regression is now canonical.
All 855 native tests, 50 unchanged reference probes and scoped Clippy pass. The
13-action reference presentation oracle remains unchanged; additive fields are
asserted separately. Failed test assumptions and isolated fixture setup attempts
are retained. Package and comparison helpers are rebuilding. See
`evidence/P14/feedback-second-improvements.json`. No new live improvement claim or
duration workload is started.

Second candidate package `7535f9958f6179c5ad619c214360a3c1bda4cdb784edfdd850aa2b5f57f485cb`
is installed and all 977 files verified. New packaged GUI open/request/render/drain,
29 release controls, four original repository adapter fixtures and exact-package
provenance pass. Fifteen rebuilt support binaries are frozen. Fresh three-model
strict canaries are running after their required full production audit, under
`.local/p14-canaries-feedback-2`. Neither held duration workload starts.
See `evidence/P14/feedback-second-improvements-installed.json`.


2026-09-09 user steering: generated cross-platform icon assets from supplied Downloads archive, rebuilt desktop in `.local/ag1-icon-build` while preserving active P14 canary inputs, installed verified root-owned payload systemwide, fixed per-user launcher precedence and shared metadata permissions, validated new native icon pixels and launch/relaunch. Full receipt: `implementation/evidence/P14/app-icon-systemwide-install.json`. Icon-source adoption remains explicitly queued until active canary settlement.

Icon follow-through: a411 live canary passed with unchanged inputs (1485.874 seconds). Adopted icon/config/packaging paths into main only after final settlement; all fingerprints match isolated icon candidate c0263d70. P14 a411 package/evidence retained as baseline, and new installed package remains separately identified.


2026-09-09 desktop startup defect: user normal project open entered unavailable TS fallback and created a misleading active ownership record. Corrected native default config lookup, project registration, preserved migrated config precedence, and shared manifest readability. 858 workspace tests + 47 cutover tests + 29 release controls pass. Installed source baf2777 / package c4da8d07, configured three preserved models, archived only the verified unstarted Rust_editor registry record. Actual installed normal-path real-model GUI check remains running; see `desktop-native-default-startup.json`.

Final installed GUI check passed with Discord left running: native project opened from saved config without env override or initialization, Qwen3.6 35B executed one native read and returned the correct README sentence, then actual UI Quit left authority quiescent and no owned processes or runtime lease. Final source12adf204/package d76077df. Startup admission remains slow (observed upper bound 207.8 seconds). No duration run. See desktop-native-default-startup.json and live-review.json.
