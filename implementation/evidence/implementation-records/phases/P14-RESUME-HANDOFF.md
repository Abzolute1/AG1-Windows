# P14 resume handoff — stopped at user request, 2026-09-08

> Historical evidence, archived 2026-09-13. Status, commands, permissions and paths below describe their original checkpoint. For current work start with [implementation status](../../../README.md). Original bytes and old paths are preserved in the [relocation manifest](../manifest.json) and recovery archive.

**P14 is incomplete. All current test work is stopped. Wait for the user to resume
work. Never start an eight-hour run without explicit authorization, or substitute
24 hours around that hold.** The user requested a prompt wrap-up, commit and handoff
so they can use the workstation. Do not automatically revive any private queue.

Read this file, [CURRENT](../../P14/CURRENT.md), the
[P14 phase](../../../../phases/P14-release-and-long-horizon-gates.md), and the historical
[P13 entry handoff](P14-HANDOFF.md). CURRENT supersedes historical running/queued
statements elsewhere. The phase requirements still govern acceptance.

## Stop receipt and exact interrupted position

[Stop receipt](../../P14/user-stop-handoff-20260908.json) binds the following:

- Waiting `.local/p14-quality-continuation-1/run.py` was interrupted first. Its
  `stopped.json` records KeyboardInterrupt before any of its four stages started.
- `.local/p14-repository-working-set-preparation/run.py .local/p14-repository/live-1`
  was interrupted next. Five complete pair-result.json files exist: m0-c0 through
  m0-c3 (35B) and m1-c0 (27B export).
- m1-c1 (27B retry repair) TS finished: original score passed, runtime settled and
  preservation verified in `ts/execution.json`. This side has not received the
  same independent semantic/final-claim review as the five closed pairs.
- m1-c1 Rust was in `client.ready()` before any protocol command. Closing stdin
  through the driver's existing finally block drained the CLI normally:
  `rust/cli-settlement.json` has exit_code 0 and cleanup_error null. No prompt was
  submitted. Retain the imported seed, private state and partial protocol evidence.
- Queue PID 771194, matrix PID 751818 and native CLI PID 782171 exited. Final process
  inspection found no repository owner/CLI, llama-server or those queue/driver
  workloads. No user Steam/browser process was touched.
- The matrix final preservation audit was not reached. There is no finished.json
  and no twelve-pair acceptance. An intentional partial run is not a model failure.

Do not reuse any existing output directory or overwrite its bound driver, scorer,
receipt or raw result. On later resume, inspect custody and validate fingerprints
first. Prepare a separately versioned continuation for the unexecuted native
m1-c1 side and remaining cases; retain explicit links to this partial matrix and
its original seed. Alternatively, declare a fresh matched rerun and retain both
attempts. Do not silently append to the old run or drop its failed observations.

## Frozen candidate and validation already done

Product input SHA-256:
`caac6ad35a7ceb86b172e630c8cb27174d1e0278df2c0e50761190539f325a54`.
Linux archive:
`.local/p14-packages/42c4bf4c242f4f3fa7ca4a66bbdbbbaa/ag1-0.1.0-rc-linux-x86_64.tar.gz`,
SHA-256 `4967cc5796e30e5a00eb0d3a4fd511d578ce31d2dec64c4c42e220a0c10b290e`.
Source and seventeen binary checkpoints are under
`.local/p14-preview-control-{source,binaries}-checkpoint/mapping.json`.

Current validation: 50 unchanged reference probes, 848 native tests, 29 Python
release controls and package frontend build passed. The unchanged frontend had
18 passing tests on the preceding candidate. Current Linux platform report binds
nine cases and 11,997 artifacts. Installed UI/CLI, preview control, paused readers,
crash/reopen, update interruptions and disposable corrupt-database refusal/recovery
are recorded in CURRENT and reports/platform-linux.json. Scripted platform tests
do not establish live quality or performance. Do not rerun broad tests merely to
recover context; verify existing artifacts read-only first.

No new product code was adopted during the final repository measurement work.
Current fixes already in the candidate include bounded context working-set reuse,
generic source/verifier/boundary guidance, duration review/cancellation controls,
and preview-control serialization/background polling/process sampling fixes.

## What the actual model evidence says

**Results vary by task and model; there is no uniform Rust coherence win.**

- Current low-thinking 35B `live-5` six-task comparison: original verifiers 6/6
  both; independent source correctness TS 5/6, Rust 6/6; final claims TS 4/6,
  Rust 6/6. Rust 24 requests/30 tools versus TS 37/37. Exact small edit diffs stayed
  in all subsequent native requests (24 checked). Large-diff pressure remains
  untested. Both owners settled; 2,497,717 distinct externalized tokens.
- High-thinking 35B original repository suite: both source contracts 4/4 and final
  behavioral explanations 4/4. Evidence requirements TS 3/4, Rust 4/4 after the
  controlled scorer corrections below. TS 29 requests/40 actual task tools; Rust
  35/46. Rust does more work here. Do not pool these with low-thinking live-5.
- 27B export m1-c0: both source contracts and exact evidence retrieval pass after
  cursor attribution correction. TS 9 requests/11 actual tools, Rust 9/12.
  **Rust final claim fails**: it says non-string fields coerce as before, but
  original Array.join emits empty fields for null/undefined and new String emits
  literal strings. The original task does not clearly require those inputs, so
  this is a contradicted model claim, not an invented source-contract failure.
  `.local/p14-repository-review-2/null-coercion/result.json` reproduces the issue.
  Rust also corrected its own initial invalid async boundary-test script.

Reports: repository-35b-four-case-review.json, repository-27b-export-review.json,
individual repository-*-pair-review.json, live5-task-review-observations.json and
live5-native-diff-retention.json. Private `.local/p14-repository-review-1` retains
source reconstruction, isolated no-network semantic checks and claim reviews.
Seven retry valid-domain checks pass both; four ambiguous invalid-input probes
fail both and were predeclared as hardening observations before the native result.

Native's initial prompt lacks the TS top-level workspace entry snapshot. Both have
same wire tool names/sampling; labels reveal adapter directories. This may affect
exploration but is only a hypothesis, not a causal result. See
repository-prompt-observations.json. No prompt fix has been prepared.

## Two measurement corrections, not product improvements

Preserve raw failed scores. The separate corrected reports explain them:

1. Native raw scorer used immutable raw journal text for a predicate that expects
   compiled externalized tool previews. The exact diagnostic was actually resident
   throughout. `.local/p14-repository-visible-score-1/score.mjs` recomputes only
   residency from actual outgoing tool messages, with negative controls and TS
   consistency. 35B retry and stale-contract Rust scores correct to pass; the TS
   export retrieval miss remains a failure.
2. Original scorer on both adapters misses artifact IDs for authenticated cursor
   reads, because arguments contain readCall rather than id. Version 2 attributes
   only reads proved by prior issued cursor, matching result ID, journal artifact
   SHA, exact byte range/payload and query. Wrong cursor/ID/bytes/range/role controls
   fail. Both 27B export scores correct to pass without changing criteria or counts.

See repository-residency-adapter-correction.json and
repository-cursor-attribution-correction.json. Version 2 private helper assumes
version 1 output names `{pair-basename}-{adapter}`; feedback reuses m0-c0 and would
collide. Before using on that new run, version a new helper accepting an explicit
version 1 input path. Never overwrite old helpers bound by accepted observations.

## Isolated draft fix saved, not adopted

The successful 27B native boundary check printed a misleading additional hint:
`No file landed at {.` A regex mistook `=>` inside quoted JavaScript passed to
`node -e` for shell redirection. Legacy has the equivalent regex issue. This is
trace-backed feedback, unlike an uncorroborated model preference.

Worktree `.local/p14-shell-hint-fix`, detached draft commit
`7ec917ba591850d616b7771324cb250c120b63d3`, contains only changes to
`product/crates/ag1-core/src/tools/shell.rs` and its tool_contract.rs regression.
A portable [patch](../../P14/drafts/quoted-shell-persistence-hint.patch) and red/
green logs are versioned with this handoff, so the fix survives loss of ignored
worktree files. Build target was `.local/p14-shell-hint-target`.

The regression fails on the original scanner; all 24 isolated tool-contract tests
pass with the draft. It skips quoted/escaped operators and conservatively avoids
inference through complex shell grammar, while retaining existing literal redirect
contracts. It is advisory hint extraction, not an authorization/effect tracker.
**Still pending:** implementation/edge-case review, formatting/Clippy, real-shell
behavior checks as warranted, and appropriate full validation before adoption.
Do not claim it belongs to the frozen 848-test candidate. If adopting later, update
source/package identity and revalidate affected release evidence; do not relabel
old live runs as testing this fix. Do not merge it merely to finish the handoff.

## Authorized work to resume only when the user resumes

1. Verify the stop receipt, immutable P13 dependency manifests, candidate identity,
   owned-runtime custody and preserved user settings. Reconcile current changes
   before any generation. Finish or explicitly replace the partial original
   3-model × 4-case matched matrix with independent source/trace/claim reviews.
2. User-requested neutral human-style model debrief is **prepared, not executed**:
   `.local/p14-feedback-preparation/run.py`, proposed destination
   `.local/p14-repository/feedback-live-1`. Scripted fixture passed, zero model
   calls. Fresh original 35B export pair, same-conversation debrief, tools disabled
   just for feedback, <=600 words. Classify each observation corroborated,
   contradicted, uncertain or suggestion; do not equate self-report to proof.
3. Original three-model context-intelligence replay via
   `.local/p14-quality-report-preparation/run.py` with a fresh output. It redirects
   report output privately to preserve the historical public report bound by old
   diagnosis. Check-only rehearsal passed; actual calls have not run in this queue.
4. Remaining fixed six-task long-horizon comparisons use
   `.local/p14-long-horizon-matrix-preparation/compare.py NEW_OUTPUT --model MODEL`:
   Qwen3.8-27B-UD-Q4_K_S and Qwen3.8-Flash-Next-AD-3.84bpw-IQ4_XS-M64. These are
   fixed functional tasks, not an elapsed-time substitute for duration acceptance.
5. Review the isolated shell hint fix and prompt/workspace-hint hypothesis in
   their own change/evidence scope. A new candidate invalidates affected old claims.
6. Formal performance is separately held during Steam download/install; last user
   report was 40%. Check actual user readiness on resume. Prepared speed and
   performance drivers have not been started by the queue. Avoid speed conclusions
   from these runs under download/build contention.
7. Duration requires explicit separate permission. 8h-1 failed ~4,109 seconds;
   8h-2 failed ~6,323 seconds with actual task/claim failures, all retained. 8h-3
   entered admission shortly before the user's hold was handled, ran zero tasks/
   protocol commands, and stopped; its missing normal drain receipts are not a
   pass. Never restart that launcher. Any authorized future attempt needs a fresh
   destination (e.g. 8h-4), full clock and workload/quality/fault evidence. Neither
   8h nor 24h is accepted.

Final P14 also needs current live quality, duration, matched packaged performance,
Windows/macOS platform evidence, signing, coverage/provenance/preservation closure
and the release gate. Windows/macOS runners and signing prerequisites remain
unavailable/unverified. Linux evidence cannot satisfy those gates. UI work does
not make the machinery proved while these gates are incomplete.

## Non-negotiable custody and backup boundaries

Accepted P13 is fca1500, 149 artifact references, native authority epoch 2 at
`/home/alex/.local/state/ag1/authority-v1`, production workspace
`/home/alex/Projects/Cruzible`, session `934360d3-bd97-4246-b48f-cee4addcf168`,
native state `.local/p13-production-20260907/native-state`. Do not rerun P13 gates
that overwrite history, remigrate, create a second production namespace, open the
original TS writer or activate stale rollback. Read-only dependency checks were
repeated successfully during P14; retain that evidence as historical verification.

Keep user profile `/home/alex/.ag1/agent/llamacpp-model-profiles.json` at
`aa02688c4d39ef73234f0ec381eed22dbe02463881626423049b75b5a12ad0d0`.
Never overwrite frozen `product/target/release/ag1-process-host`, SHA
`e1c7ca1711b2b7834eea1c99a55fe958cb4cfb880ed3c6909b06232c36bc1da2`.
P14 binaries use `.local/p14-target`, `.local/p14-quality-target` and separate
private build directories. Preserve tuned backends/libraries/weights/arguments.
Private capture `.local/p13-runtime-capture-20260907/configuration.json` SHA
`b6ea6f521e34c0278dd5d3c097ca2c83036eeebd52cb49305c4623734f3a9fba` binds
111 materials / 124,130,526,787 bytes. Do not print its private environment.
Original pins: Cruzible e30f0b4b238ee5618928bcd17b4571eb046e7038;
Dyad fb17039bfd2adcac7d74b9c1178a3a3524df35b6 (upstream skeleton, AG1 machinery).

The earlier backup commit cd682ffb8591bd21068232ffbe9c7c39703960d5 was pushed
and verified on private Abzolute1/AG1. This wrap-up requests a local commit, not a
new push. No token is stored or repeated. Ignored `.local` evidence, runtime
materials and models are not in Git: preserve them on disk. The versioned draft
patch is portable; public reports still depend on referenced local raw artifacts.
