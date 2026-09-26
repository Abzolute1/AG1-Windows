# P14 live UI audit — source repair receipt

> Historical evidence, archived 2026-09-13. Status, commands, permissions and paths below describe their original checkpoint. For current work start with [implementation status](../../../README.md). Original bytes and old paths are preserved in the [relocation manifest](../manifest.json) and recovery archive.

Date: 2026-09-09. Source fixes implemented and validated with controlled local tests. **Not installed or live-model accepted.** The original `P14-LIVE-UI-AUDIT-HANDOFF.md` remains the frozen investigation record.

## Repairs

| Handoff | Source behavior now |
| --- | --- |
| A, L | Preparation retains its bounded deadline. Human review has a separate five-minute expiry. Accepted, revalidated approval issues a separately bounded execution capability; argument, scope, revision, cancellation and replay fences remain. Changed files are rejected after review before execution. Approval dialogs show normalized proposal excerpts, countdown/recovery text, an explicit deny-on-close action, and no Enter-to-approve behavior. Pending policy choices are labelled for the next response. |
| B, C | Native checks, shell and jobs share configured executable paths, compiler selection, rustup configuration and project-scoped Cargo cache/target paths. Rustup automatic installation is disabled. Native Linux sandboxes mount resolved CA bundle files read-only and set the verified bundle paths without disabling TLS verification. |
| D | `editPreview.newText` is required and must be a string **before scalar coercion**. Missing/null/numeric values produce correction guidance. Explicit empty-string deletion remains valid. Catalog guidance describes `editApply`'s path plus ordered edits and distinguishes preview from authorization. |
| E, H | Delegation budgets follow child limits with bounded settlement time. Exploration uses registered endpoint capacity; queued local workers do not start their child clocks. All-failed, partial, completed and cancelled groups report distinct outcomes, and incomplete tasks are errors. Each request emits preparation/admission phases; generating tool arguments no longer means a tool is executing. |
| F, G | Mutable memory is projected at the request tail with evidence/data labelling. Its revision still participates in admission validation, but does not invalidate eligible compiled history solely for memory projection. Admitted compiler history now remains stable below pressure; continuation reductions prefer newer eligible evidence and stop at low water. Final compiled requests and prefix-change diagnostics are retained separately from precompiler inputs. Compaction remains bounded and enabled. See the context follow-up below. |
| I, J, K | Editor markup matches the flex/toolbar styles. Monaco fills the remaining panel and resizes. Visible Files gets bounded refresh and a manual control, with response/scope guards and preservation of unsaved content. Native preview hunks render as literal readable diffs, with a raw fallback and explicit Output/Preview headings. Collapsed results retain lazy loading and bounded rendering. |
| M, N | Todo queries return the normalized list and stable IDs. Lifecycle conflicts identify the item and recovery path. Reads expose canonical source-file references and digests; source-reference errors explain the syntax and distinguish source hashes from formatted output. Observation/memory presentation distinguishes recorded claims from verification. |
| O, Q | Programming queries with years avoid current-events classification. Fetch coverage describes evidence without directing premature task completion. Builtin plan submission has a recognized policy classification; extension name collisions remain rejected. |
| P, R | Missing owned web-preview targets explain the supported Runtime/Vite setup. Existing stale-edit, atomicity and inspection-repetition protections remain. A later follow-up adds bounded native recovery for repeated stale edits, checklist loops, missing executable calls and failed delegated tasks; see below. Generic static HTML has not been added as an owned browser target. |

## Validation completed

All providers used in session tests were controlled loopback fixtures. No live model request was sent.

- Offline `cargo check` for core/platform passed.
- Core P14 regressions: 5 passed; existing approval identity/expiry/replay regression: 1 passed.
- Platform P14 environment regressions: 3 passed. These include actual `cargo test --offline` in the sandbox shell, `cargo build --offline` in jobs, and native Cargo checks with both real binaries and rustup shims. The isolated project has no external dependencies; paths contain spaces.
- P14 native session regressions: 2 passed. They cover late approval, denial, source changes during review, one terminal result, per-round phases, omitted/null preview replacements, and explicit deletion previews without publication.
- Working-set contracts: 8 passed, including memory-tail updates, unchanged system prefix, branch isolation, discarded preparation, source freshness and restart reconstruction.
- Delegation contracts: 18 passed; the added all-failed/mixed/completed exploration test also passed. Local queue/cancellation expectations were updated to assert that queued children have not started.
- Focused existing native regressions passed: approval commands (2), repetition (4), surgical edits (2), failed atomic edit family (1), memory (7), todo (2), and in-flight memory/source admission invalidation (1).
- Frontend full suite: 56 passed. After the final frontend edits, typecheck and the 27 affected App/ToolActivity tests passed again.
- Real Chromium/Monaco fixture checks passed at 2476×1356, 1000×700 and 640×480. Editor dimensions were respectively 1981×1245, 505×589 and 405×369, matching the available panel. The mobile sidebar was closed using its normal control. A file refresh preserved the unsaved editor text and selection.
- Changed-file whitespace checks passed. Repository-wide `git diff --check` still reports pre-existing trailing whitespace in `product/apps/desktop/src/generated/protocol.ts:35`; that file was preserved.

Browser probe: `tools/probes/p14_ui_audit.mjs`. It uses an in-memory desktop bridge, refuses unexpected mutations, and requires a local frontend server on port 1429. `AG1_PLAYWRIGHT_MODULE` can point to an installed Playwright module. Its screenshot is `.local/p14-live-fixes/resumed-editor-browser.png`. The owned validation server was stopped afterward.

## Preservation and limits

Before editing, the current tracked/untracked source, fixtures, tools and implementation files were archived to `.local/p14-live-fixes/resumed-source-20260909.tar.gz`. Comparison afterward found **17,066 identical baseline files, 30 intentionally changed baseline files, and no missing baseline files**. New repair files are additional to that archive. The archive is a preservation aid, not an instruction to restore the workspace wholesale.

The installed release remains `/opt/ag1/releases/2ebdc491fb814b959b79378f24c62e01`. These protected hashes still match the handoff:

- `product/target/release/ag1-process-host`: `e1c7ca1711b2b7834eea1c99a55fe958cb4cfb880ed3c6909b06232c36bc1da2`
- Model profiles: `aa02688c4d39ef73234f0ec381eed22dbe02463881626423049b75b5a12ad0d0`
- Preserved capture configuration: `b6ea6f521e34c0278dd5d3c097ca2c83036eeebd52cb49305c4623734f3a9fba`

Rust validation used `.local/p14-target` with `CARGO_INCREMENTAL=0`; no `product/target` rebuild or cleanup occurred. Approximately 1.5 GB remained free after checks. The Trailbook audit project, transcripts, installed application and model settings were not modified.

Not claimed: live-model reliability, measured cache-hit improvement, public-registry dependency acquisition, Windows/macOS execution parity, full historical differential-suite acceptance, actual 165 Hz delivery, or completion of the earlier Trailbook project. Empty-cache acquisition from a controlled HTTPS registry is validated in the later TLS follow-up below. Approval previews are bounded excerpts; complete arguments remain in the native tool record. General static HTML ownership was not introduced. Semantic-retry work was left for the follow-up recorded below.

The changed research guidance, preview validation and memory placement deliberately differ from historical SDK behavior. Historical oracle files were preserved; they are not new-change acceptance receipts.

No installation, release replacement, live inference test or eight-hour run was performed. Live inference and duration runs still require explicit user authorization.

## Context-stability follow-up — 2026-09-09

Implemented after the user authorized continuing the context fixes. This completes the source-level reduction-stability work left open in the initial repair pass; it is **not** live-model/cache-performance acceptance or completion of the separate broader semantic-retry work.

### Changes

- A continuation compiler takes the previously admitted compiled-prefix length only after the working-set owner/epoch, configuration, branch ancestry and exact original-message prefix have passed validation. Below context and metadata pressure, old admitted bytes no longer change solely because steering or subsequent rounds move the recent-turn window.
- At renewed context pressure, continuation reduction processes newer eligible evidence first, stopping historical payload/output and thinking reduction at the existing low-water target. Existing artifact previews are not repeatedly rewritten into another summary format. Metadata pressure can still evict eligible metadata; exact artifact custody, recent-turn protections, evidence leases and tool-call/result pairing remain. If protected content cannot fit, over-target statistics and the hard admission margin remain authoritative; the compiler does not pretend it fitted.
- Cold reconstruction and arbitrary extension-context callbacks retain the existing compiler behavior; delegated sessions retain their separate hygiene path. The historical compiler oracle is unchanged and passes. Continuation state still publishes only after successful admission and is not a durable-history replacement.
- Final compiler/provider-input diagnostics now record the admitted/unchanged historical prefix, configuration equality, common message-prefix length and first differing message index relative to the previous admitted request. Comparisons use bounded per-message digests; the retained envelope includes the final messages, system, tools, model and thinking setting. These are compiler/provider-input comparisons, **not** tokenized HTTP-wire or KV-cache measurements.
- Added `runtime-request-compiled` to the native custody filter. Audit envelopes remain directly retrievable but no longer appear as ordinary evidence-search results and recursively feed copies of old prompts back into context.

### Offline validation

- New core continuation contracts: 5 passed. Coverage includes repeated large mutation rounds, exact Unicode/CRLF payload recovery, no-change idempotence, steering, changing fixed/tail budgets, metadata pressure, active leases, artifact rehydration, recent thinking, invalid prefix lengths, failed durable storage and irreducible protected pressure.
- Existing core context contracts: 12 passed, including all sixteen historical cold/warm compiler captures.
- Native working-set contracts: 9 passed. The new controlled-loopback scenario executes twenty distinct file reads over twenty-one requests: 6 reduction rounds and 14 unchanged-history continuation rounds. It verifies multi-round reduction headroom, compares prefix diagnostics with actual prepared messages, reads back every retained final request, and checks that durable original tool results remain intact. These are synthetic compiler results, not live cache measurements.
- Focused native session filters passed: compaction (17), context (9), memory (7), artifacts (7), overflow (4), delegation (19), and in-flight memory/source invalidation (1). Filters overlap; these counts are not a distinct-test total. They include callback bypass, cancellation/drainage, branch/model changes, restart reconstruction and hard context admission.
- Storage context contracts: 6 passed; custody/evidence-search filtering regression: 1 passed.
- Changed-file whitespace checks passed. Shared test-support unused-function warnings remain non-fatal.

### Preservation and remaining acceptance

Immediately preceding this follow-up, relevant source/tests and this receipt were preserved in `.local/p14-live-fixes/context-before-20260909.tar.gz`; the subsequently identified storage paths were separately preserved in `.local/p14-live-fixes/context-storage-before-20260909.tar.gz`. Existing work was not restored, reset or cleaned. The installed release link and all three protected hashes listed above were rechecked and remain unchanged. Tests used `.local/p14-target` with `CARGO_INCREMENTAL=0` and `--offline`.

No application installation, model configuration change, live inference, GPU benchmark or eight-hour run occurred. Actual provider cache reuse, fresh/cached token counts and time-to-first-output still need a separately authorized live comparison. Policy-sensitive controlled-runtime system guidance was not moved or weakened. No universal prefix stability is promised when configuration, branch, capacity, protected evidence or extension transformations legitimately change the request.

## Bounded retry-recovery follow-up — 2026-09-09

Implemented after the user authorized the next retry-loop fix. This is deterministic native recovery for recognized cases, not a claim to detect every semantically equivalent natural-language task or shell program.

### Behavior

- Existing inspection deduplication and source-revision checks remain unchanged. Their historical SDK trace test still passes.
- A separate bounded, session-owned recovery tracker observes acknowledged native results, including pre-execution rejections that never reached the existing inspection hook. It does not count replayed receipts, unsettled executions, extension-projected output or failed durable publication. It stores at most 64 hashed recovery cases; unrelated narration/bookkeeping does not erase them.
- Three recognized failed edits against the same file revision pause the response. Paths are normalized; changing stale oldText/replacement wording does not evade the case. A valid corrected edit still executes normally; changed source or a successful mutation resets the relevant history. Exact matching, approval, atomicity and source fences are unchanged. Recovery guidance now also covers editApply/editPreview, and no-op edits explain that unchanged replacements should not be resubmitted.
- Three native checklist lifecycle failures against unchanged checklist state pause. No-argument queries return IDs/statuses without erasing the failure history. Three redundant successful updates after the baseline declaration also pause as **no-progress updates**, not failed storage. Meaningful checklist changes and valid corrections reset the relevant history; persisted successful results remain successful.
- Repeated missing executable invocations pause after three actual native exit-127/command-not-found results. Classification is intentionally restricted to simple executable invocations; distinct arguments, quoting, pipelines and arbitrary shell programs are not merged. Existing raw-output custody and one-time missing-tool warnings remain.
- Repeated settled `task` failures are keyed by task brief, profile, actual/requested model, checkpoint and budget settings, excluding the UI description. Changed labels alone do not evade the guard; different briefs, checkpoints or budgets are fresh recovery cases. Task timeout/budget/cancellation outcomes and partial exploration are not blindly classified as identical failures.
- The existing durable repetition-pause path reports that the task is incomplete, identifies the last native result and gives a concrete recovery action. Later mutations in that batch are rejected before preparation/approval/execution. A new user instruction resets the live guard. No automatic edit, installer, retry or success claim is introduced.

### Validation

All model endpoints were controlled loopback fixtures; no live inference was used.

- Recovery state/classification unit tests: 3 passed, including the 64-case bound, revision/success reset, narration-independent keys, changed checkpoints, and successful-poll/result distinctions.
- Native repetition tests: 8 passed. These include stale argument variants, exact recovery after one failure, source changes, checklist queries/conflicts/redundant updates, later-batch mutation rejection, pause commit/ack faults, and new-user resume. The stale-edit fixtures explicitly enable the edit tool.
- Missing-tool recovery tests: 3 passed. Native expectations now end at the third failed command; the pure historical failure-tracker oracle remains unchanged.
- Delegation tests: 20 passed, including a new three-failed-child test with changing labels, plus existing checkpoint, cancellation, effect-replay and drainage checks.
- Surgical edits (2), native todo lifecycle/atomicity (2), approval commands (2), P14 preview/review regressions (2), and context working-set regressions (9) passed. Test groups overlap where explicitly rerun; no full-suite total is claimed.
- Changed-file whitespace checks passed. Existing shared-fixture unused-code warnings are non-fatal.

### Preservation and limits

Pre-edit files are retained in `.local/p14-live-fixes/retry-before-20260909.tar.gz`, with the subsequently touched failure/delegation tests in `retry-failure-tests-before-20260909.tar.gz` and `retry-subagent-tests-before-20260909.tar.gz` in the same directory. No existing work was reset, cleaned or restored wholesale. The installed release link and protected process-host/profile/capture hashes still match the identities above. Offline Cargo tests used `.local/p14-target` and `CARGO_INCREMENTAL=0`; approximately 1.2 GB remained free.

Recognition deliberately fails open for unrecognized errors and unverifiable file revisions. At this stage, extension-enabled native batches bypassed the new tracker and compiler, complex-shell and exploration coverage remained open; the next follow-up addresses these cases. Correction to the earlier platform limitation: the native session, workspace, file tools and project runtime are gated to Linux, so Windows/macOS support needs a broader platform port, not just a file-revision helper. Counts are live working control state, not a new durable failure ledger. Live reliability, cross-platform acceptance and performance measurements remain outstanding.

No release installation, live inference test or eight-hour run was started.

## Expanded retry coverage — 2026-09-09

### Changes

- Native compiler checks now expose their existing lifecycle and structured diagnostics in result details. Three repeated failures at an unchanged source revision pause with repair guidance. Diagnostic identity includes the reported file/range/code/message; changed diagnostics or source are fresh cases. Results whose source changed during execution are excluded. When diagnostics reach parser size/count bounds or have no located compiler error, classification requires complete retained-log hashes instead.
- Complex shell failures use the exact admitted command (including configured native transforms), timeout, actual exit status and complete retained output hash. Different commands or output remain distinct; output excerpts, incomplete retention and non-exit termination do not establish an identical failure. Previous command effects are not undone or declared unsuccessful merely because the response pauses.
- Repeated all-failed exploration groups are matched by questions, worker model/state and complete error digests, excluding per-run IDs and timing. Full errors remain in the retained result artifact rather than being duplicated in context. Different failures remain distinct; partial/completed groups reset the streak, while cancellation and timeout states are not treated as ordinary repeated failures.
- Built-in recovery now also works with extensions enabled. It reads the actual admitted arguments and native result artifacts after durable acknowledgement; extension display projections cannot hide native failures or manufacture them from successful execution. Custom extension tools are not assigned invented native recovery semantics. Existing replay, approval, source-validation and later-batch pause fences remain.

### Validation

All Cargo invocations used `--offline`, `.local/p14-target` and `CARGO_INCREMENTAL=0`. Model requests went only to controlled loopback fixtures; child-provider failures were simulated without inference.

- Recovery unit contracts: 5 passed, including exact shell/output identity, incomplete-output exclusion, changed compiler diagnostics/source, capped-diagnostic hash fallback, differing worker failures and partial-exploration progress.
- New real compiler/session contract: 1 passed. Three actual Cargo type errors pause; a new user instruction permits a source repair and a passing check. The fixture supplies Cargo's canonical lockfile so initial lockfile normalization does not masquerade as an unchanged source revision; the production freshness fence was preserved.
- Extension contracts: 29 passed, including native failure hidden by a success projection, native success displayed as failure, and transformed shell inputs.
- Delegation contracts: 21 passed, including repeated all-failed exploration with retained worker outcomes and bounded error identities. Rerun after the final error-digest change.
- Existing repetition (8), missing-tool/failure (3), approval (2), P14 session audit (2), and context working-set (9) contracts passed.
- Project runtime contracts: 21 passed, covering diagnostics, source/runtime ownership, failure/cancellation, isolated build/browser fixtures and process settlement. This is not installed-application or live-model acceptance.
- Changed-file whitespace checks passed. Existing shared-fixture unused-code warnings remain non-fatal.

### Preservation, limits and next step

Pre-edit files are retained in `.local/p14-live-fixes/retry-coverage-before-20260909.tar.gz`. Existing work was not reset, restored wholesale or cleaned. The installed release link and all three protected hashes above were rechecked unchanged. No installed application, model configuration or frozen audit evidence was modified. About 762 MB remained free after validation; no disk cleanup was authorized or performed.

Windows/macOS execution parity remains a separate native-runtime port: the session, workspace, filesystem tools and project runtime are Linux-gated. No isolated helper was added or represented as cross-platform support. The retry guard deliberately does not claim arbitrary shell-program or natural-language equivalence, durable cross-restart retry accounting, or live reliability.

Next scoped step: add a controlled TLS dependency-fetch regression starting with an empty isolated project Cargo cache, verifying both trusted-certificate success and untrusted-certificate rejection without disabling TLS verification. Check disk headroom before running further build fixtures; any material cleanup needs separate approval. No live inference or eight-hour run is part of that step, and both still require explicit authorization. No release installation occurred in this pass.

## Empty-cache TLS validation — 2026-09-09

The controlled dependency-fetch gap above is now covered. This pass adds regression coverage, not another production behavior change: the existing CA-resolution/mount repair passed the real native check.

- A loopback HTTPS sparse registry serves one tiny generated Rust dependency and its checksum. Each trusted/untrusted case starts with a separate project and no project Cargo cache. Registry replacement points only to that local server; no public registry or model endpoint is needed.
- A child mount namespace supplies a test-only `/etc/ssl/cert.pem` symlink pointing outside `/etc/ssl`. The unmodified native runtime resolves the link, mounts the canonical bundle read-only, sets its ordinary Cargo trust environment, downloads the dependency and compiles successfully. A second native check with `--offline` succeeds from the same isolated cache; the server observes only one package download.
- A separate fresh project trusts an unrelated test certificate. Cargo exits 101 with a certificate-verification diagnostic, the server receives no HTTP request, and no downloaded-crate cache is created. TLS verification is never disabled. The fixture accounts for TLS 1.3 peers that close after server handshake completion but before sending HTTP.
- Test certificates/private keys, registry archive, project/cache and state exist only in owned temporary directories. The outer test namespace has the host filesystem read-only except the fixture directory; the host trust store is not edited. The loopback server is stopped and joined when the fixture ends.

Validation: `cargo test --offline --manifest-path product/Cargo.toml -p ag1-platform --test p14_live_audit` passed all 4 substantive tests. One internal child-worker test is ignored in the outer suite and explicitly exercised twice by the TLS contract. The suite also reruns the real shell/job/native Cargo environment and symlink/missing/invalid trust-file contracts. Rust validation used `.local/p14-target` and `CARGO_INCREMENTAL=0`; only the generated fixture's initial Cargo check enables network access, to loopback HTTPS. Changed-file whitespace checks passed.

Preservation: `.local/p14-live-fixes/tls-before-20260909.tar.gz` retains the pre-edit audit test and receipt. Production source, frozen handoff/evidence, installed release and model configuration were not changed in this pass. The release link and all three protected hashes still match the identities above. About 758 MB remains free; no existing build cache or user files were cleaned. No live inference, release installation or eight-hour run occurred.

Remaining work is primarily acceptance, not a newly established list of code defects. Next: a release-readiness review of the accumulated changes and acceptance coverage, without installing or starting inference. Actual model/cache behavior, short live UI acceptance and performance claims remain unverified; live tests and any eight-hour run require explicit authorization. Windows/macOS native-runtime support remains a separate substantial port. Public-registry/proxy behavior is not established by this controlled TLS fixture, and the frozen Trailbook project is still not claimed complete.

## Release-readiness review — 2026-09-09

Verdict: **not release-accepted**. This was a bounded review of the repair receipt, approval/continuation/retry safeguards and packaging entry points, not an exhaustive audit of the pre-existing dirty worktree. No release gate, packaging command, install, model request or duration run was started.

### New finding: expiry text and Allow state can disagree

`ApprovalReview.tsx` owns a local one-second countdown, while the Allow button in `App.tsx` evaluates `Date.now()` only when its parent renders. An unchanged pending snapshot, without a running elapsed-time clock, can therefore show the expired message while leaving Allow enabled. Unchanged polling responses are deliberately deduplicated and do not guarantee a parent render. This is a UI-state defect, not an authorization bypass: the native authorizer still rejects `now >= binding.expires_ms` and retains its exact binding/revision checks.

A temporary controlled React regression reproduced the defect: the request begins enabled, its expiry text appears after advancing fake time, and the expected disabled button remains enabled. The probe failed its final assertion (`expected false to be true`). The exact probe and pre-review receipt are preserved in `.local/p14-live-fixes/readiness-expiry-probe-20260909.tar.gz`; only the newly created temporary probe was removed from the normal suite afterward. No existing test or production code was changed to conceal or repair the finding.

Next fix: give the approval countdown and action state the same expiry clock, including unchanged/reconnected snapshots, and recheck the deadline at click time. Preserve full-request review, deny/close behavior, keyboard safeguards, binding identity and native enforcement. Turn the retained probe into a permanent regression and test expiration, replacement requests and unmount cleanup before claiming this follow-up complete.

### Validation and outstanding gates

- Frontend typecheck and the existing full suite passed: 56 tests across 10 files. The additional diagnostic probe failed as described above; the green existing suite did not cover that expiry transition.
- Repository-wide whitespace checking still reports the pre-existing trailing space in `product/apps/desktop/src/generated/protocol.ts:35`; it was preserved.
- The installed release remains the pre-repair release. Existing historical acceptance receipts do not certify this changed source. Packaging and exact-candidate validation still need adequate disk space (approximately 757 MB currently free), explicit scope and preserved rollback identity; no cleanup was performed.
- Controlled tests establish the listed source contracts, not actual model/cache reuse, live UI reliability, 165 Hz delivery, full cross-platform parity or completion of the frozen Trailbook project. The short live acceptance script in the frozen handoff remains pending explicit authorization; duration tests are separate and also unauthorized.
- A private virtual display can be useful for future functional UI acceptance, but it must expose the real desktop app and keep it mapped/visible inside that display. A hidden/minimized browser is not equivalent: this app explicitly slows polling when `document.hidden`, and physical display/focus/frame-delivery behavior needs separate acceptance. No virtual display or live test was started during this review.

## Approval-expiry UI repair — 2026-09-09

The expiry-state finding from the preceding readiness review is now repaired in source after user authorization.

- The review text and action button share the same component-owned deadline clock; unchanged host snapshots no longer leave Allow enabled after the countdown expires. The clock updates at most once per second, schedules the final partial-second deadline, and stops scheduling after expiry. It does not cause a periodic rerender of the entire application.
- A click checks the actual current time and immediately updates the same dialog state before attempting approval. An expired click sends no native command, including when background timers have not fired. The submission path separately rechecks the deadline. Missing/non-finite deadlines cannot enable an action.
- Focus and visibility changes refresh the clock. Replacement request identities initialize a fresh review; deadline changes replace the timer. Unmount removes the timer and both event listeners.
- Full-request paging, select-answer validation, exact binding identity, Deny/close behavior and Enter-to-approve prevention remain. No native authorizer, source/replay fence, execution lifetime or backend approval policy was changed. The wrapper remains a React fragment, preserving the existing dialog element structure.

Validation: frontend typecheck passed; all 67 frontend tests across 11 files passed. The 11 new permanent regressions in `product/apps/desktop/src/ApprovalExpiry.test.tsx` cover the retained unchanged-snapshot reproduction, the exact expiry boundary, delayed-timer clicks, replacement/truncated requests, timely exact-binding submission, Enter prevention, Deny/close, deadline replacement, visibility refresh, timer/listener cleanup and invalid deadlines. Existing full-request paging and the other 56 tests still pass. Changed-file whitespace checks passed. Existing Node deprecation/localStorage warnings are non-fatal. This was controlled React/bridge testing, not live desktop or model acceptance.

Preservation: the pre-edit `App.tsx`, `ApprovalReview.tsx` and receipt are archived in `.local/p14-live-fixes/approval-expiry-before-20260909.tar.gz`. Comparison against that archive confines production edits to approval presentation/submission. Existing work, frozen audit evidence and installed application remain intact. The installed release link and all three protected process-host/profile/capture hashes were rechecked unchanged. No Rust rebuild, installation, live inference or duration run occurred.

Next step: prepare acceptance of the repaired desktop candidate, first checking disk headroom and the available isolated UI-control setup. A test against the unchanged installed release would not validate these source repairs. Actual candidate build/install scope must be agreed before release replacement; live inference and any eight-hour run still require explicit authorization. No additional code defect is established by this pass, and no full release-acceptance claim is made.

## UI acceptance preparation — 2026-09-09

The current approval renderer passed an additional real Chromium probe with an in-memory bridge: automatic expiry, replacement requests, exact-binding approval, close denial, Enter prevention, literal preview text and dialog fit at 1440×900 and 900×600. Screenshots were inspected. The passing receipt and screenshots are in `.local/p14-live-fixes/approval-browser-fibZEZ/`; the reusable probe is `tools/probes/p14_approval_ui_audit.mjs`. Its owned loopback server and browser were stopped. No native backend or model was contacted, and no production code was changed in this pass.

Native preparation found an available private Xvfb executable with resolved dependencies, but the existing desktop binaries predate the repairs. Only about 759 MB is free, so no native rebuild was attempted. [Native acceptance preparation](P14-NATIVE-ACCEPTANCE-PREP.md) records the setup limits and an exact six-file **recoverable compressed-archive cleanup proposal requiring approval**. No cleanup, installation, private-display launch, live inference or duration run occurred. The release link and all protected hashes were rechecked unchanged. Pre-edit receipt: `.local/p14-live-fixes/ui-acceptance-prep-before-20260909.tar.gz`.

Next decision: authorize that bounded archival space reclamation and an isolated candidate build, or provide another storage plan. Live inference and release replacement remain separate approval boundaries.

## Recoverable cleanup, isolated build and native startup — 2026-09-09

Following user approval, the six exact old test executables in the preparation proposal were compressed, verified and their uncompressed copies removed. They remain recoverable from `.local/p14-live-fixes/test-binary-archives-I9CBe8/`; `RESTORE.md` records hashes, modes, sizes and non-overwriting restoration instructions. Net reclamation was 1,905,856,996 bytes. A failed `fuser --` compatibility check did not prevent the first three already archive-verified removals; work was paused and the process check corrected before the other three. This limitation is retained in the manifest. No broader cache cleanup occurred. The user separately deleted a game; about 142 GB is now available, so storage is no longer the build blocker.

The current renderer and diagnostic native desktop built successfully into `.local/p14-live-fixes/native-candidate-08EBCR/`. Rust used offline locked dependencies, `.local/p14-target`, `CARGO_INCREMENTAL=0`, debug profile and the feature-gated visual-test broker. Build commands/logs and binary hashes are in the candidate README. Existing renderer output was preserved and compared unchanged. Pre-build renderer/docs and debug binaries are archived in `.local/p14-live-fixes/native-build-before-20260909.tar.gz` and `native-debug-binaries-before-20260909.tar.gz`.

The bounded `tools/probes/p14_native_startup_smoke.mjs` passed in actual Linux Tauri/WebKit on an authenticated, non-TCP private Xvfb display. The fixture has separate application state and XDG browser data/config/cache, with only a loopback rejecting provider endpoint. Passing evidence: `native-candidate-08EBCR/startup-pYpWS1/result.json`, plus visually inspected home/Settings screenshots. The current embedded renderer, 1440×900 viewport, safe approval default, fixture metadata and Settings navigation passed; zero prompts or provider requests occurred. Owned desktop/display processes were stopped. Earlier fixture-model assertion failures exposed shared WebKit preferences in the initial probe; a wait alone did not help, and private XDG storage corrected the harness without a production edit.

The installed release link and protected process-host/profile/capture hashes were rechecked unchanged. Source work was not reset, and frozen audit artifacts were not edited. This pass added diagnostic tooling/documentation, not another production repair. It does not establish native approval workflow acceptance, live model/cache behavior, installation readiness or performance. No installation, live inference or duration run occurred.

Next step: broader bounded native UI/approval workflow tests with an isolated fake provider against this candidate. Real-model testing, any eight-hour run and release replacement still need explicit authorization. See [updated native acceptance preparation](P14-NATIVE-ACCEPTANCE-PREP.md).

## Controlled native approval regression acceptance — 2026-09-09

The isolated candidate now passes real native-host/WebKit approval workflows through `tools/probes/p14_native_approval_audit.mjs`, sharing the startup harness. Final evidence: `.local/p14-live-fixes/native-candidate-08EBCR/approve-17bSfK/`. The candidate binary/embedded renderer identity is unchanged from the preceding build.

Four scripted-provider prompts verify actual file effects and provider-visible native results: Allow publishes exact reviewed bytes; Deny and close preserve original bytes; changing the target during review preserves the external edit and rejects the stale write with a fresh-edit diagnostic. Exactly eight loopback provider requests and one terminal tool result per case are asserted. Additional direct native-IPC checks reject altered argument hashes and consumed-binding replay specifically as `invalid_approval`, with fresh envelope revisions. UI checks cover literal HTML-like content, cancellation of synthetic Enter by the real handler, and dialog fit at 1440×900 and 900×600. Screenshots were inspected. This is diagnostic DOM-driven UI coverage, not physical-input or live-model acceptance.

The test uses a new disposable project, private AG1/XDG state, read-only host-root mount, shadow native authority registry and private PID/tmp namespace. It cannot write existing host project/source/configuration files through that mount. Owned app/display/server processes were stopped. No real model endpoint, install or duration workload was started.

The first four-case probe passed (`approve-menIff`). An intermediate expansion failed a Send-button readiness assertion (`approve-iZtE9U`); native completion had preceded renderer polling. A bounded readiness wait fixed the driver, not production code. The expanded suite passed (`approve-k36Bpg`) and was rerun with exact rejection-code/single-result assertions to produce the final receipt above. The startup-only CLI also passed again (`startup-SDtBjj`, zero prompts/requests). `npm test -- --run src/ApprovalExpiry.test.tsx` reran all 11 previously added expiry regressions successfully, including the original unchanged-snapshot failure reproduction. Both probe syntax checks and changed-file whitespace checks passed. No full backend/frontend suite rerun is claimed in this test-only pass.

Pre-edit harness/docs are preserved in `.local/p14-live-fixes/native-approval-before-20260909.tar.gz`. Existing production work, installed release and protected process-host/profile/capture identities remain unchanged. No new production defect was established by this bounded pass.

Next: native editor layout and visible-file refresh/dirty-buffer regressions on disposable fixtures. The wall-clock native expiry/cancellation paths are not covered by this new suite; controlled clock/backend and renderer expiry receipts remain separate evidence. Live model/cache behavior, installed acceptance and physical-display performance remain unverified and require their separately agreed scope.

## Native editor acceptance and two follow-up repairs — 2026-09-10

The requested native editor pass found two additional production defects in the preceding diagnostic candidate. Neither was concealed by the existing 67-test frontend suite, which passed again before this investigation:

1. After a conflicting save, Discard followed by reopening the same file updated React's file bytes/revision but did not reset Monaco's retained model. The editor continued showing the old text. `Editor` intentionally does not consume every text prop because that would overwrite concurrent edits; `readFile` was missing an explicit reset after an accepted read. Native reproduction: `.local/p14-live-fixes/native-candidate-08EBCR/editor-IpSZd2/failure.json` and `failure-view.json`. A permanent React regression, using a retained-model editor mock instead of an always-controlled textarea, also failed with expected `externally changed`, received `original`.
2. The dirty marker was inside the ellipsized filename span. A long name pushed it outside the visible title, hiding unsaved status. Native reproduction: `native-candidate-08EBCR/editor-YQrZvI/dirty-marker.json`; its right edge was approximately 1294 px while the title ended at 753 px in a 900 px window.

Repair: an accepted explicit `readFile` now advances the editor reset key, after the existing scope and intervening-edit guards. Background refresh does not reset the model. The filename has its own ellipsized child and full-path tooltip; the dirty marker is a non-shrinking sibling. Production changes are limited to this read/reset handoff and scoped file-toolbar markup/styles. Byte comparison against the pre-edit archive verified that all other existing `App.tsx` and CSS changes remain intact.

Validation:

- All **69 frontend tests** across 11 files passed, including the formerly failing same-file reload regression, a new delayed-reread/intervening-edit safeguard, and existing failed-save/later-keystroke/approval-expiry tests. Typecheck passed. The first new test used an unsupported testing-library `exact` option; it was corrected and typecheck rerun successfully.
- Renderer and offline locked debug native builds passed into a **new** diagnostic candidate: `.local/p14-live-fixes/native-candidate-SKS6XR/`. The previous candidate and its failed receipts remain unchanged. Native build uses `.local/p14-target`, `CARGO_INCREMENTAL=0`, default custom-protocol and `visual-test`; no installed release was rebuilt/replaced.
- `tools/probes/p14_native_editor_audit.mjs` passed against that new candidate: `editor-T4WBjK/result.json`. Real WebKit/Monaco fills the pane at 1440×900 (945×789 editor), 900×600 (405×489), and after hiding the sidebar (665×489). Long names do not overlap toolbar buttons; the dirty marker is visible. Inspected screenshots include large layout, small dirty/conflict state and final saved content.
- Automatic create/rename/removal refresh and explicit refresh preserve the selected dirty model. Creation was observed after about 3.3 seconds. A fixture file was moved out of the project to test disappearance without deleting its bytes. Dirty file-switch refusal, stale-save rejection, external-byte preservation, Discard/reopen recovery and exact successful save all pass. **Zero prompts and provider requests** occurred in this editor suite.
- The previous native approval suite was rerun on this exact new candidate and passed: `approve-JxpbTd/result.json` (four scripted-provider prompts, eight loopback requests; no real model). Both suites use disposable projects/state, private browser storage, read-only host root and shadow authority registry. Owned application/display/server processes were stopped.
- Probe syntax and changed-file whitespace checks passed. Initial editor-driver failures were a quoted-newline syntax error and an assertion that did not account for Monaco soft wraps; these harness problems were corrected before the separate production failures above were established. Exact save checks still compare disk bytes, not normalized display text.

Preservation: `.local/p14-live-fixes/editor-reload-before-20260910.tar.gz` contains the pre-fix source/tests/CSS and probe; `native-editor-before-20260909.tar.gz` retains the earlier receipt/preparation documents. Existing desktop `dist`, previous candidate binary, installed release link and protected process-host/profile/capture hashes are unchanged. No user project, frozen audit evidence, installation, live inference or eight-hour workload was modified/started.

Next scoped follow-up: make the file-conflict message explain recovery clearly. The native screenshot still exposes raw `{"error":"Conflict"}` plus a command ID; preservation/recovery works but the message does not tell the user how to proceed. Native chat/cancellation, live-model/cache behavior and full release acceptance remain separate coverage gaps, not claims established by this editor pass.

## File-conflict recovery message — 2026-09-10

The last confirmed follow-up in the current repair list is now complete. `bridge.ts` translates an exact native `files/save` rejection/failure with `data.error === "Conflict"` into a recovery explanation: the disk file changed, unsaved text remains in the editor, copy it somewhere safe before Discard, then reopen the file from the file list and reapply changes. The command ID remains for diagnostics. Other operations/error payloads retain their original diagnostics. No automatic retry, force-save, file reload, buffer mutation, backend revision change or new permission is introduced.

Reproduction and validation:

- Two new message tests first failed against the old raw JSON output; retained log: `.local/p14-live-fixes/conflict-message-before-test.log`. Both were rerun successfully after the change.
- All **78 frontend tests** across 11 files pass, including nine new cases: both rejected/failed conflict replies, five unrelated-error cases, success-response preservation, and editor-level dismissal/refresh without buffer loss or resubmission. The previous same-file reload, dirty-marker-related UI behavior, delayed read/save and approval tests remain in the passing suite. Typecheck passes. Initial fixture IDs did not satisfy the inferred UUID string type; the test IDs were corrected and typecheck/tests rerun without widening production types.
- A new isolated diagnostic build, `.local/p14-live-fixes/native-candidate-DQhmfU/`, embeds `/assets/index-wqSugjQ_.js`. Desktop SHA-256: `4174f914774bb323007df481bbad56aba7f55992226063e75e5b9ff19179c69b`. Renderer/typecheck and offline locked debug native builds pass; commands, identities and logs are retained in its README. Previous candidates and existing desktop `dist` remain unchanged.
- Native editor/recovery suite passed on that exact candidate: `editor-Og4WxR/result.json`. It now asserts the human-readable message and absence of raw error JSON, then verifies unchanged local dirty text/external disk bytes and successful Discard/reopen/reapply/save recovery. The 900×600 conflict screenshot was visually inspected; the explanation wraps without hiding the editor or dirty marker. Zero prompts/provider requests occurred in this suite.
- Native approval suite rerun passed: `approve-zKcI6M/result.json` (four scripted prompts/eight loopback requests, no model). Both suites include startup checks, use private state/browser storage/authority and a read-only host root, and stop owned application/display/server processes.
- Changed-file whitespace and probe syntax checks passed. Comparison against the preservation archive confirms the only production-source change in this pass is the scoped error-formatting block in `bridge.ts`.

Preservation: `.local/p14-live-fixes/conflict-message-before-20260910.tar.gz` retains prior bridge/source tests, editor probe and receipts. Protected process-host/profile/capture hashes and installed release link were rechecked unchanged. No installation, live inference, eight-hour run or user-project modification occurred.

**Remaining confirmed fixes in the current repair list: 0.** This is not a claim of zero undiscovered bugs or full release acceptance. Next bounded step is native chat-switching/cancellation validation with controlled fixtures. Native wall-clock expiry, actual model/cache behavior, physical-input/display performance, cross-platform execution and release acceptance remain separate validation/support gaps. Real-model tests, eight-hour runs and installation still require explicit authorization; no new code defect is asserted solely from an untested area.

## Native chat switching and queued cancellation — 2026-09-10

Confirmed repair backlog remains **0**; this pass adds bounded validation, not a production change. `tools/probes/p14_native_lifecycle_audit.mjs` passed on the unchanged `native-candidate-DQhmfU` desktop. Final receipt: `.local/p14-live-fixes/native-candidate-DQhmfU/lifecycle-T2KT0v/result.json`; provider observations and checks are in `provider-lifecycle.json`. An earlier supported-case run also passed (`lifecycle-ZvbEYN`); the final rerun strengthens the history/state isolation check across the entire outgoing message array.

The native UI opens a second conversation in the same disposable project while A streams. B honestly shows queued status behind the single local endpoint slot. Stopping queued B makes no B provider request and does not stop A. Switching restores each conversation's separate unsent draft; Settings does not cancel A. The scripted server then completes A normally, releasing its slot; a fresh prompt in B completes successfully without A's history/request-local state appearing in B's request. Three UI prompts produce exactly two loopback requests, because the queued prompt is cancelled before dispatch. Sentinel file bytes remain unchanged. Queued/recovered screenshots were visually inspected. Timings in the receipt are fixture observations, not performance claims.

Important failed-attempt accounting and limit:

- `lifecycle-XneXbH` assumed both chats could reach the provider simultaneously and timed out waiting for B. Source inspection confirms fixture/loopback endpoints are classified Local and intentionally have capacity one. The corrected test asserts the queue instead of changing that scheduler.
- `lifecycle-pFIpyU` used an attempted JavaScript invoke wrapper that did not observe native cancellation calls. That unreliable observer was removed. The passing native test instead proves selected B settles while A's same operation keeps streaming; the existing React exact-scope/operation cancellation regression was separately rerun and passed (one selected test; 28 others skipped).
- The attempted dispatched-stream cancellation case, `lifecycle-BNXO1j`, **did not pass**. HTTP transport closed, but native completion timed out; the UI retained “Stopping response…” and reported Conflict. This plain HTTP fixture has no `GenerationRuntime`/owned runtime shutdown proof. `cancel_generation` therefore cannot settle a local generation permit; a disconnect alone must not release it. That explains why this fixture cannot establish active-cancellation recovery, but does not establish that every real owned-runtime recovery path is correct. No quarantine, revision or ownership safeguard was weakened, no elapsed-time settlement was added, and this failure is not counted as a successful cancellation test. The planned before-first-output cancellation case was not reached and remains pending.

The final reusable probe covers the supported queue/navigation contracts only and explicitly excludes post-dispatch cancellation recovery. All owned application/display/server processes were stopped, including after failed attempts. Failure receipts capture their pre-cleanup state; a `closed: false` field there is not a post-cleanup process inventory. New-probe syntax/whitespace checks passed. No new build, full frontend/backend-suite rerun, installation, live inference or eight-hour workload occurred in this pass. Pre-edit receipts: `.local/p14-live-fixes/native-lifecycle-before-20260910.tar.gz`. Candidate binary, installed release link and protected process-host/profile/capture hashes remain unchanged.

Next: a controlled runtime-aware cancellation fixture that supplies genuine owned-process shutdown evidence, then rerun dispatched-stream and before-first-output cancellation/recovery. Do not add a fake success acknowledgement or call real-model inference to fill this gap without authorization. Native approval expiry, actual model/cache behavior, physical input/performance and release acceptance remain separate pending areas.

## Owned-process native cancellation validation — 2026-09-10

Both previously unverified dispatched cancellation cases now pass with genuine fixture-process custody. Confirmed production repair backlog remains **0**: this pass adds diagnostic composition/coverage, not a change to production cancellation logic or a real-model acceptance claim.

`cancellation_fixture.rs` is compiled only with `visual-test`. Its explicitly selected fixture controller launches the diagnostic executable's own fixed-response loopback HTTP helper with a cleared environment, retains the actual Child handle, and accepts cancellation only after kill plus wait. It rejects non-fixture/tuned-runtime/credential-bearing/wrong-model/non-loopback configurations. The helper cannot launch a model or arbitrary command. Its bounded lifetime is a failure condition, never settlement evidence. Default builds exclude the module and composition/CLI branches; their cargo check passes. Existing quarantine, ownership, generation-permit and revision behavior is unchanged.

New candidate: `.local/p14-live-fixes/native-candidate-NFhH7R/`, desktop SHA-256 `b4ab427904dc59c5ae321c3c5a0911b18d41ab6cd3ef90d87611192b7eef8bd3`, embedded renderer `/assets/index-wqSugjQ_.js`. The renderer was freshly built/typechecked in the preserved preceding attempt `native-candidate-ieXMTN`, then copied unchanged. Offline locked debug build and all commands/identities are recorded in the new candidate README.

`tools/probes/p14_native_owned_cancellation_audit.mjs` passes twice: `owned-MAUuCR/result.json` and `owned-mvdvwQ/result.json`. Each run sends four fixed prompts, producing four requests. For both visible streaming and a dispatched request with no response headers/output, Stop kills/reaps the actual server, native state settles, the stopping indicator clears, unsent draft survives, and a fresh request in the same conversation completes on a replacement child. Exact event/PID/status evidence is in each `runtime-events.jsonl`; PIDs are namespace-local. Sentinel bytes remain unchanged. Streaming/recovered screenshots from MAUuCR were visually inspected. Timings are fixture observations, not real-model latency benchmarks.

Failed-attempt accounting remains explicit:

- `native-candidate-ieXMTN/owned-YkPGmV` failed because the new fixture parser assumed the last user-role message was the human prompt; runtime context follows it. The fixture rejected that context before either cancellation case. The parser now selects the latest exact scripted prompt and has a permanent regression test. Failed binary/receipt remain preserved.
- `native-candidate-NFhH7R/owned-JhPy3M` reaped the server and settled natively, then failed an immediate renderer assertion before the next polling update. A bounded wait for actual UI settlement corrected the driver; both full reruns pass. No forced UI state or production polling change was made.
- Earlier DQhmfU `lifecycle-BNXO1j` is still a failed plain-HTTP test without owned shutdown proof. It is not relabelled as passing. New valid owned-runtime cases fill the scoped coverage gap; ignored-abort quarantine remains intentionally enforced.

Regression reruns on NFhH7R: native queue/navigation `lifecycle-axs5Ot`, native approvals `approve-fC7SrK`, native editor/recovery `editor-E1eMB5` all pass. All **78 frontend tests**, **6 desktop library tests** (including two new fixture tests), and **20 backend contracts** across client conversations, provider HTTP and runtime custody pass. Negative unproven-abort/quarantine contracts and positive owned-stop contracts are included. Probe syntax and changed tracked-file whitespace checks pass.

Preservation: `runtime-fixture-before-20260910.tar.gz` retains pre-edit source/helper/documents. Prior candidates and failure evidence are intact. Existing desktop `dist` matches its preservation archive; installed release link and protected process-host/profile/capture hashes are unchanged. All owned test processes stopped; post-run inventory was empty. No installation, real-model inference, eight-hour run, user project or frozen audit changes occurred. This remains diagnostic DOM-driven native WebKit testing, not physical-input/display or release acceptance.

Next bounded check: native wall-clock approval expiry and recovery with a controlled provider. Actual model/cache behavior, physical-input/performance, cross-platform and release acceptance remain separate gaps; no authorization for live inference, installation or duration testing is implied.

## Compact approval dialog — 2026-09-10

User-requested visual improvement, not a newly discovered execution defect. The approval popup now has an approval-specific 480px layout, smaller spacing/type hierarchy, a scrollable request body and a separate always-visible decision footer. Actual request details (including commands/paths), proposed changes and critical outside-project warnings remain visible; only explanatory approval/keyboard help is collapsed. The live countdown is quiet for screen readers; expiry is still announced and checked again on click. Unknown or oversized structured titles retain the literal fallback.

No authorization policy, binding, target/source revision, one-use consumption, Enter suppression, denial-on-close or truncated-request full-review gating was relaxed. The pre-existing dirty worktree was preserved. The previously requested per-conversation folder grants/primary-folder workflow has only been investigated and is **not implemented** by this UI change. That feature remains next; general website screenshot support is excluded per the user's direction.

Verification: all **84 frontend tests** pass (78 existing plus 6 compact-review regressions); TypeScript and isolated renderer build pass. Diagnostic candidate `.local/p14-live-fixes/native-candidate-MfokDE/` built offline/locked with `visual-test`, desktop SHA-256 `16e9822903b9488e95afa1c901df63e89b2cd5fd0f3584dac5e2aa0ed3f532c0`, embedded entry `/assets/index-C0HAeGDF.js`.

Native WebKit approval probe passes in `approve-RDzdsG/result.json` and again in `approve-85VbzM/result.json` after extending the driver with a long proposed write. Each run uses four fixed prompts/eight scripted loopback responses, no model. Approve, deny, close, changed-source rejection, altered-binding rejection and consumed-binding replay rejection pass. Layout checks cover 1440×900 and 900×600, collapsed/expanded help and long content with unobstructed decision buttons. Collapsed short review measures 480×390. Screenshots `review-900.png`, `review-help-900.png` and `review-long-900.png` were visually inspected. This is DOM-driven native diagnostic testing, not physical-input acceptance or native wall-clock expiry coverage; existing deterministic expiry tests were rerun and pass.

No installation, live inference or eight-hour workload. Installed current remains `releases/a459017613ba4372a84f2dafece16dc8`; pinned release process-host, model profile, frozen runtime capture and native configuration hashes match their prior values. Owned test processes stopped. The installed app does not yet contain this UI change.

## Per-chat folder access — 2026-09-10

Implemented the previously requested GUI workflow, following the compact approval dialog. Open **Chat folders** from the chat header or add menu. Add a folder by path or the Linux native Browse picker, select Read only (default) or Read & write, change its mode, make it the working folder, or revoke it. Picking a path does not itself grant access. Changes preserve the conversation journal, selected model and unsent draft. The editor, relative-path tools, Git, context/resources and project tools follow the selected working folder; additional folders support absolute-path file/search access. No startup flags or website screenshot feature were added.

Enforcement and recovery:

- Grants are conversation-local, persisted atomically in private state, revision-bound and pinned to directory identity. Explicit approval policy still applies. Stale UI scopes/runners, replaced directories, overlapping roots, protected private storage and more than 16 roots are rejected. Missing roots are marked unavailable instead of silently authorizing a replacement.
- Capability changes rebuild the same conversation runtime and invalidate old runners. Active responses, queued work, live jobs, open manual terminals, active project processes and unsaved editor changes prevent unsafe switching. Stop/approval/recovery operations remain available when a directory disappears.
- File tools enforce read-only/read-write grants; agent shell writes use pinned read-only/read-write mounts, including paths containing spaces. Autonomous shell policy recognizes explicit grants without accepting ungranted sibling prefixes or weakening planning mode. **Existing agent-shell host-wide reads remain; these grants are not a full read sandbox.** Manual terminal and trusted plugin privileges are separate and disclosed in the dialog.
- Reopening after a missing primary uses an empty private recovery root, never an implicitly reauthorized former project. The user can select another available working folder and revoke/re-add the missing one. Project-runtime locks are released when switching away and on host shutdown.
- The native picker uses GTK's asynchronous dialog future to retain ownership until response; dropping a native dialog immediately after `show()` would not retain a window-system reference. Picker selection/cancellation wiring is frontend-tested and the native implementation compiles; physical native chooser interaction is not claimed by the DOM probes.

Verification receipts are under `.local/p14-live-fixes/`:

- **91 frontend tests / 12 files pass**, including six folder-manager tests and an exact-chat-scope queue test (`folder-frontend-verified.log`). Renderer/typecheck build passes.
- Two folder unit tests pass (`folder-units-final.log`), and both native client contracts pass (`folder-contract-final.log`). These exercise actual file/search/shell effects, read-only denial even with approvals disabled, read/write grants, revocation, scoped epochs, per-chat isolation, live-job rejection, persistence, missing-directory recovery, history preservation, and reopen/working-folder changes. No real provider/model is used.
- The new explicit-folder shell-policy test passes (`folder-core-policy-final.log`). Original policy/approval comparisons also pass. Default desktop compilation and the isolated diagnostic native build pass.
- Candidate `native-candidate-Q2MtVw` passed folder, approval and editor/recovery native WebKit probes (`folders-4LvDNi`, `approve-vDuWxY`, `editor-BMejLZ`). Folder testing preserves history/draft, changes the actual editor root, revokes access and checks the 900×600 layout. Its screenshot was visually inspected. These use a private authenticated Xvfb display, private application/authority state, read-only host root and scripted loopback responses, not real-model inference or physical-input acceptance. The subsequent picker-lifetime correction is covered by the final candidate receipt below.

Failed-attempt accounting:

- `folder-recovery-tests.log` exposed a real retained project-writer lock after reopening. Cleanup was corrected; the exact contracts were rerun and passed (`folder-recovery-rerun.log`, then final contracts).
- An early frontend suite hit the existing dirty-editor timing assertion. The exact failed case and subsequent full suites passed without changing production editor behavior.
- **Six historical comparison failures remain**, and were rerun, not relabelled as passing: four core `tool_contract` cases compare old preview validation/research guidance; two platform cases compare old file output without source-attribution metadata. The old SDK fixtures are deliberately preserved. Logs: `folder-core-rerun.log`, `folder-platform-regression.log`, `folder-legacy-read-rerun.log`, `folder-legacy-files-rerun.log`. The core run has 21 passing/4 failing cases; platform tool contracts have 32 passing/2 failing. These mismatches concern earlier implementation changes, not six newly demonstrated defects in folder access. Full-suite green or release acceptance is **not** claimed.

Final candidate: `native-candidate-Rp85Q8`, desktop SHA-256 `3fe4f293fc7359747add003a4b7e52e0d58810024440bc79c152ae3fd18f9bc0`, embedded entry `/assets/index-DMA3vd_K.js`. After the picker-lifetime correction, **all three native probes pass**: `folders-8XtHES/result.json`, `approve-E3WYkJ/result.json`, `editor-lkltny/result.json`. Default desktop check also passes (`folder-desktop-default-check-final.log`). An unsupported reflink copy and first missing-renderer probe setup were corrected; failed setup evidence is retained in the candidate README and `folder-audit-missing-renderer.log`. Scoped tracked-file whitespace and probe syntax checks pass. No owned candidate/display processes remain.

Preservation: existing dirty work, earlier candidates, frozen audit and user projects remain intact. Installed release, pinned production helper, model-profile, frozen runtime configuration and native configuration identities were rechecked unchanged. This feature and the compact approval UI are **not installed systemwide**. Next step is an explicitly authorized build/install for user testing; live inference and eight-hour workloads remain unauthorized.

## Local image previews and model/harness handoff — 2026-09-10

Implemented inline tool-image results, automatic restricted-vector SVG-to-PNG previews through `read`, original SVG downloads, corrected native tool guidance, and text-only-model continuation without losing UI image attachments. Raw diagnostics remain collapsed; no general website screenshot capability was added. Full implementation, safety boundaries, passing tests and failed-attempt accounting: [P14-LOCAL-IMAGE-PREVIEW.md](P14-LOCAL-IMAGE-PREVIEW.md).

95 frontend tests and focused SVG/provider/prompt/folder contracts pass. Native image/create/download/zoom, folders, approval and editor probes pass on candidate `native-candidate-QF6dsP`. The prior six historical comparison mismatches persist; the corrected static-server guidance adds one deliberate reference-wording difference. Frozen oracles remain unchanged, and full release acceptance is not claimed. Existing work, installed app and protected runtime identities are preserved. No live inference or duration run. Next step remains an explicitly authorized build/install for user testing.
