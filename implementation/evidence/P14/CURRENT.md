# P14 execution checkpoint — user duration hold, 2026-09-08

P14 remains incomplete. **Do not start or restart an eight-hour workload until the
user explicitly instructs it. Do not substitute a 24-hour workload around this hold.**
**All current test work is now stopped at the user's request for a handoff.**
Read [P14-RESUME-HANDOFF.md](../../P14-RESUME-HANDOFF.md) before further work.
The functional queue was interrupted before any follow-up stage. The repository
matrix stopped after five complete pairs, with m1-c1 TS complete and native
interrupted during admission before any protocol command. Native CLI drained with
exit code 0 and no cleanup error; no matching workload process remains. No final
matrix audit or full-matrix acceptance is inferred. See user-stop-handoff-20260908.json.
The historical functional-continuation-queue.json is a start receipt, superseded by
this stop. Do not restart its launcher automatically. Duration still needs explicit
user direction. Formal performance remains held while Steam download/install is
active (last user report: 40%). Overlapping timing is descriptive only.

The armed 8h-3 launcher entered admission about 61 seconds before the hold took
effect. It was interrupted before any task or protocol command; all known owned
process identities exited. Normal duration-result/drain receipts were not written
after the enclosing launch session ended, so no graceful-drain pass is inferred.
See duration-8h-attempt-3-user-hold.json. Future duration requires explicit user
direction and a fresh destination/full clock. No accepted eight-hour or 24-hour run exists.

## Current candidate and validation

Frozen source: `caac6ad35a7ceb86b172e630c8cb27174d1e0278df2c0e50761190539f325a54`.
Linux archive SHA-256: `4967cc5796e30e5a00eb0d3a4fd511d578ce31d2dec64c4c42e220a0c10b290e`.
Exact source and seventeen binaries are retained in preview-control checkpoints.
Fresh current differential passes 50 unchanged reference probes and 848 native
tests. All 29 Python release controls and the package frontend build pass;
unchanged frontend tests passed on the preceding working-set candidate.

The accepted P01–P13 evidence remains immutable. Fresh coverage mapping verified
those dependency manifests and maps current tests across 127 parent/invariant/
defect/llama requirements and 44 child families with 149 cases. Final release,
live and performance dispositions remain pending. Production native authority,
original reference pins, user runtime profiles and tuned backend bytes are preserved.

| Area | Current result |
| --- | --- |
| Provenance | Exact current Linux package and separate extension closure reviewed; reports/provenance.json validates. Other OS packages and signatures remain separate. |
| Installed Linux | CLI protocol, real project install/run/stop, stale-session refusal, WebKit preview/error/close/reopen, quit-cancel, crash/relaunch and four update interruption boundaries passed. |
| Archive recovery | Current installed CLI and matching native administrative helper passed deliberate disposable corruption refusal and restoration of the exact conversation and completed receipt. The nine-case reports/platform-linux.json validates 11,997 artifacts. No inference during restoration or production authority transfer. Initial fixture omitted host.json and was refused; failed attempt retained. |
| Paused CLI | Actual 45.21-second reader stall, full durable text and completed receipt recovered, zero leaks. See paused-installed-cli.json. |
| Paused renderer | Actual identity-checked WebKit pause for 45.69 seconds. Native completed while paused; unchanged frozen pixels, repaint after resume and full 520,343-byte response recovered. Peak conservative owned Unix IPC backlog 227,960 bytes under an 8 MiB fixture budget, zero leaks. See paused-production-renderer.json. |
| Shell payload size | Current packaged tree 111,103,820 bytes versus packaged Electron reference 382,479,337 bytes: 70.95% smaller. OS libraries separate; direct external ELF itemization is 263,378,928 bytes and is only a prerequisite floor. No fresh-machine total-size or timing claim. See installed-shell-size-current.json. |
| Coverage | Current 848-test source mappings verified. See child-coverage-current.json. |

GUI/platform checks use scripted providers and private authenticated Xvfb. They do
not establish live model quality or formal performance. GUI5's initial project hit
its existing five-minute execution deadline during manual inspection; that raw
observation and explicit restart remain retained. Preview close/reopen was then
verified while the project was ready, without QueueFull. See
preview-control-installed-validation.json and preview-poll-contention.json.

## Actual current model comparison

Live-5 is fully closed on the preserved Qwen3.6-35B-A3B configuration and 2,497,717
distinct externalized tokens. Both TS and Rust passed all six original verifiers.
Independent source/semantic review passed five of six TS tasks and all six Rust
tasks. Final-claim review passed four of six TS tasks and all six Rust tasks.
All failed baseline observations remain retained. Both owners settled and full
preservation/fingerprint checks passed.

TS still returns an empty route for empty/whitespace input, violating the leading
slash requirement despite passing the original three-case verifier. Rust handles
those cases correctly. TS's batch implementation is correct but its final table
inverts the invalid-size guard; Rust describes its implementation accurately.
Rust read each verifier before editing. Rust used 24 provider requests and 30 tool
calls, versus TS 37 and 37. No fair speed advantage is inferred under these conditions.

All six native tasks reused the admitted context working set, with source-freshness
checks retained. Neither owner called unavailable taskState in this current pair.
The independent supplemental cases were run by Codex after the model tasks; they
were not extra tests performed by the models. Native backoff reasoning included an
intermediate imprecision about the old expression, retained in the review; its
final source and explanation passed. These findings do not establish uniformly
better reasoning or sustained long-session coherence.

See live5-legacy-baseline-review.json, live5-task-review-observations.json and
long-horizon-live-preflight-5.json. The first current repository pair is closed: Rust retrieved required historical
evidence and passed; TS produced correct code but skipped required retrieval. Both
final code/claim reviews passed. Rust corrected regex mistakes and used 9 requests/
12 task tool calls versus TS 8/10. See repository-first-pair-review.json; this pair
does not show a general efficiency advantage.

The second repository pair also has correct source and final explanations in both
adapters. Both retain the exact retry-delay diagnostic in every outgoing request;
both pass seven reviewer-only valid-domain checks. Four exploratory invalid-input
checks fail in both, retained as robustness observations under the same scope policy
recorded before the Rust result. Rust uses 11 requests/16 task tools versus TS 8/13.
The native raw score incorrectly marked resident evidence absent because the private
adapter supplied immutable journal text to a scorer expecting compiled messages.
The corrected measurement uses actual outgoing tool messages with unchanged original
criteria, preserves the raw failure, and reproduces the TS score. Negative controls
still reject missing exact evidence. No product or active matrix input was changed.
See repository-second-pair-review.json and repository-residency-adapter-correction.json.
The third pair also passes source and final-claim review in both adapters: the
current session-key contract wins over obsolete history. Four supplemental checks
pass for each. Both use six requests; TS uses eight task tools and Rust nine.
Rust needs no artifact read because the exact current rule remains resident.
Its raw residency failure is corrected by the same independently checked adapter.
The native final table has an unescaped pipe cell, a retained presentation defect.
See repository-third-pair-review.json. The fourth pair passes clamp behavior and
zero irrelevant-history retrieval in both adapters. Rust's final change-size wording
is imprecise, but its behavioral explanation is accurate.

All four repository cases for the 35B model are now reviewed: both adapters have
correct source and behavioral explanations in all four. Original evidence criteria
pass TS 3/4 and Rust 4/4 after the separately documented visibility measurement
correction. TS uses 29 requests/40 task tools; Rust uses 35/46. This set supports an
evidence-compliance advantage, not uniformly better reasoning or efficiency. It is
a high-thinking comparison and is not silently pooled with the low-thinking live-5
tasks. See repository-fourth-pair-review.json and repository-35b-four-case-review.json.
Four of twelve repository pairs are reviewed; the 27B and Flash matrices follow.
The first 27B legacy task also has correct code and explanation, including a real
cursor read of the required historical export contract. Its raw failure exposes a
second scoring blind spot: the old attribution code reads IDs only from call
arguments, while cursor reads return the ID in their result. A separate adapter
checks the owner-issued cursor, returned identity, original source hash and exact
returned bytes containing the contract before assigning credit. Five negative
controls reject unissued/tampered/mismatched/non-tool evidence; all eight earlier
side results remain unchanged. Raw reports stay preserved. See
repository-cursor-attribution-correction.json. This is not a product fix or a new
model request. The matching native task is pending.
The rest of the original full-session
repository and context-intelligence matrices remain necessary before broader quality
conclusions.
The earlier context-intelligence matrix remains a retained failure: TS 18/18,
Rust 17/18, including malformed JSON on Qwen3.8-27B. It is not current acceptance.

## Adopted corrections and retained endurance failures

Context working-set retention preserves only the last admitted canonical projection
with constant-size SHA-256 ancestry/original-prefix witnesses. It does not rewrite
durable records. Fresh compilation checks lease, source freshness and admission.
Selected source-mutation tools receive general verifier/contract-boundary guidance,
with no fixture answers. Actual HTTP/session tests cover reuse, stale admission,
branch navigation, changed model/memory, source freshness, leases and reopen.

The duration driver now stops new activity after an explicit bound failed review
and handles operator interruption while draining the client. Missing or partial
reviews cannot grant acceptance. Preview controls await the existing serialization
lock, recheck scope and perform the callback once; disappearing process samples
are handled. All current differential checks pass.

Attempt 8h-1 failed after 4,108.69 seconds on a taskState echo loop. Its selected
tool catalog lacked taskState while prior guidance instructed use. Raw requests,
cancellation investigation, cleanup and preservation audit remain retained.

Attempt 8h-2 failed after 6,322.56 seconds and 30 completed original repairs.
Route tasks 3, 9, 15, 21 and 27 violate the empty-input contract. Task 28 prematurely
claims an edit before its tool call; its eventual code and final answer pass.
All six failed reviews remain binding. Fork, branch, resume, actual renderer
reload/reconnect and corrupt-state/archive recovery completed. Scheduled model and
host crashes were not reached. All owned processes settled, three CLI drains
returned zero, and the full preservation audit passed. The old raw driver's
misleading awaiting-review status is retained and corrected by the failure report.
See duration-8h-attempt-2.json. Neither short attempt is a duration pass.

Historical live-4, preceding working-set canaries and all prior source/binary/public
snapshots remain retained with their original fingerprints. The 36 three-model
strict canaries used source 641e74a0 before the narrow preview/sampler correction;
they are not represented as executions of the current candidate.

## Remaining work and backup

1. Complete and independently review the original repository matrix, current
   context-intelligence matrix and remaining two-model long-horizon comparisons.
2. After an uncontended measurement window, run original speed probes and the
   prepared packaged startup/display, idle shell and alternating core measurements.
3. Only after explicit user instruction, start fresh full eight-hour and 24-hour
   workloads with every required fault and per-lifetime memory fit.
4. Assemble final coverage/preservation and exact-candidate reports; verify all
   immutable artifacts and run the P14 gate when its required inputs exist.
5. Windows/macOS native runners and project signing identities remain unverified
   or unavailable. Linux results cannot replace those gates. See platform-prerequisites-after-backup.json
   for the refreshed repository/runner/signing distinction.

GitHub backup was pushed and remote-verified at commit
`cd682ffb8591bd21068232ffbe9c7c39703960d5` in private `Abzolute1/AG1`. It contains
source/history/versioned reports at that snapshot. Ignored private raw evidence
and models remain local. No token was persisted. Repository availability does not
establish native OS runners or signing facilities.

The reference is retained. UI work does not replace these machinery and release
gates. No public release is authorized.
