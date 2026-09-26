# AG1 live UI audit — detailed repair handoff

> Historical evidence, archived 2026-09-13. Status, commands, permissions and paths below describe their original checkpoint. For current work start with [implementation status](../../../README.md). Original bytes and old paths are preserved in the [relocation manifest](../manifest.json) and recovery archive.

Date: 2026-09-09. Status: **investigation complete; repairs NOT implemented**.

## 1. Read this before doing work

The user's final instruction was to **stop implementation and write this handoff only**. Do not interpret this document as authorization to change code, rerun inference, start an eight-hour workload, build, or install. Those are future steps when the user resumes the work.

The brief repair phase was cancelled before any source or test mutations. All three agents confirmed read-only activity. The operator compared all 575 files in the pre-repair snapshot with the workspace and found no differences. No new checks, build, installation, or post-repair live run occurred. The installed AG1 remains the version tested below. The live test did create and modify the isolated Trailbook project; that is distinct from modifying AG1.

The goal of a future repair is reliable, understandable behavior, not making every tool result appear green. Models can produce invalid code or arguments. The harness must expose the right tools/environment, reject incomplete requests clearly, preserve evidence, report partial failure accurately, and help the model recover without loops.

## 2. What was actually tested

The installed desktop application was driven with ordinary mouse, keyboard and clipboard interactions on physical **workspace 5**, as explicitly requested. The operator submitted a coding task to the selected **Qwen3.6-35B-A3B-UD-Q4_K_XL** model and reviewed approval dialogs, navigation, tool output and progress. A separate agent read native journals, logs and source. It did not control the UI, send inference requests, or modify the project.

This was a roughly 83-minute exploratory run, not an eight-hour run, benchmark, or release acceptance. It required substantial operator steering to recover from environment failures and repeated model mistakes. Do not present it as autonomous project completion.

Project: `/home/alex/Tests/AG1-Trailbook-20260909`.

Requested project: a local-first developer worklog with a Rust CLI, durable JSON storage, add/list/done/tag/search/export operations, meaningful tests, a self-contained HTML dashboard, example data, documentation, surgical follow-up edits, and broad tool coverage. AG1 authored the project files. The operator did not silently repair its source or run its compiler outside AG1.

Observed outcome:

- CLI `cargo build --offline` passed with zero errors and eight warnings after explicit environment guidance. A background `job` build also exited zero after the same environment correction.
- Source files, tests, dashboard, example JSON, README and model-authored coverage notes were created.
- Direct edits made real changes. A correctly specified preview followed by `editApply` also succeeded, but required explicit operator-supplied argument guidance.
- **No passing final test suite was established.** Earlier test compilation failed; the last corresponding edit was not followed by a completed successful test run.
- Browser screenshot tools did not obtain a valid preview target. No model-driven browser/vision verification was completed.
- The operator ended the owned test application using SIGTERM on PID 826185. A last bash call lacked a terminal journal result at the frozen snapshot. Treat it as an operator-ended test interruption, not another demonstrated tool defect.

### Backend identity and performance evidence

Read-only `/props`, `/v1/models`, runtime process arguments and native logs agreed on:

- Model: `Qwen3.6-35B-A3B-UD-Q4_K_XL`.
- Weights: `/home/alex/.lmstudio/models/unsloth/Qwen3.6-35B-A3B-nonMTP-GGUF/Qwen3.6-35B-A3B-UD-Q4_K_XL.gguf`.
- Server: `/home/alex/Projects/llama-cpp-turboquant/build/bin/llama-server`.
- Multimodal projector configured; server reported vision enabled.
- Context 90,000; parallel 1; batch 256; ubatch 128; threads 12; GPU layers 99; CPU MoE setting 20. These were the existing profile, not tuning performed during this test.
- Warm decoding was approximately 40–50 tokens/second. A long generated file can therefore take significant time even without a harness stall.
- Initial observed preparation included approximately 38 seconds before server start, 12.2 seconds loading weights, and 35.1 seconds evaluating 12,129 prompt tokens. These phases must not be conflated.
- One memory mutation caused a later request to evaluate **34,617 fresh tokens with zero reported cache reuse**, taking approximately **98.9 seconds**, while the same model weights remained loaded.

The monitor was 2560×1440 at 165.002 Hz. **Actual application frame delivery was not measured.** Do not claim a 165 Hz pass from monitor configuration or screenshots.

## 3. Failure accounting: harness versus model

Frozen native snapshot: **136 tool calls, 135 results, 51 results flagged failed, 84 not flagged failed**. Failures comprise 44 parent-session and 7 delegated-session results. One final call was unfinished when the operator ended the test.

| Primary descriptive bucket | Failed results | Interpretation |
|---|---:|---|
| Rust/toolchain environment | 3 | Installed Rust was not exposed correctly to the generic shell. |
| Approval expiry | 8 | Review and execution shared a 60-second lifetime. Operator review/focus delays contributed; the harness timing design made those delays consume execution time. |
| Delegation deadline/cascade | 8 | One parent task timed out; seven downstream operations were cancelled. These are not eight independent defects. |
| Model arguments or generated code | 18 | Six compiler/test-code failures, eleven invalid/stale/lifecycle/no-op calls, and one unrelated unavailable-language probe. |
| Enforced policy boundaries, mixed causes | 11 | Includes correctly blocked installer attempts and attempts to work around hidden toolchains. Do not assign the entire bucket to either side. |
| Missing browser preview prerequisite | 3 | No ready owned web-preview grant; error text did not explain recovery. |

The buckets sum to 51. They identify the primary observed reason for each failed result, not exclusive causal responsibility. For example, model retries can be triggered by a harness environment defect, and an approval expiry can reflect both a slow review and a poor deadline model.

Additional unsuccessful behavior was **not flagged as failure**:

- **Nine `editPreview` calls omitted `newText` and produced unintended deletion previews.** The model supplied incomplete arguments; the schema/adapter accepted omission as deletion. None of those previews applied a deletion.
- **Two `explore` wrappers returned success although all six requested workers failed.** This overlaps the downstream cancellation count; do not add it as six new independent failures.
- A successful `job poll` reported command exit 101. Polling succeeded; the command did not. This distinction is legitimate and must remain visible.
- Accepted memory/runtimeObservation records contained model-authored evidence mistakes. Storage success is not independent verification of the claim.

The earlier conversational count of 50 was a previous snapshot. A later no-op edit added the 51st failure. Exact call IDs, classifications and diagnostics are preserved in [failure-attribution.json](../../../../.local/p14-live-project/failure-attribution.json).

## 4. Highest-priority repairs

### A. Separate approval review lifetime from tool execution time

**Observed:** writes, edits, bash and a job expired before execution while the operator reviewed the application or approval. The UI continued showing a review dialog without an understandable expiry/recovery state. This is a direct cause of the apparent “edit tools do not work” experience.

**Source path:** `product/crates/ag1-platform/src/client/mod.rs` around line 400 sets the default timeout; `src/session/files.rs` around lines 164–238 creates the deadline before adapter preparation and review, then uses `tool_timeout` for the approval binding lifetime; `src/session/approvals.rs` handles expiry; `product/crates/ag1-core/src/tools/approval.rs` validates/consumes bindings. Line numbers are investigation anchors, not stable API identifiers.

**Proposed correction:** represent preparation, human review, and execution as separate phases with explicit budgets. Begin the execution budget only after an accepted, revalidated approval. Use a separately bounded review lifetime and a visible expired state. Preserve all existing binding checks: exact arguments, operation/call identity, scope, policy revision, source/target revision, cancellation, one-time consumption, and replay prevention. Do not merely extend every timeout or remove expiry checks. Preparation can be expensive and needs its own cancellation/budget treatment.

**Later tests:** approve after more than the old 60 seconds and verify the full configured execution budget remains; deny/cancel/expire and verify no file changes; change source or policy during review and verify rejection/re-review; reject replayed approval; confirm only one terminal result; inspect the expired dialog and keyboard behavior. Use a controllable clock for deterministic tests instead of long sleeps.

### B. Expose configured toolchains consistently to shell and background jobs

**Observed:** ordinary `cargo`/`rustc` failed despite Rust being installed. The model concluded Rust was absent and attempted provisioning; those installer attempts were blocked before execution. Explicit real compiler paths and environment variables eventually worked. The same issue recurred in a background job.

**Source:** `product/crates/ag1-platform/src/client/tooling.rs` around line 115; `src/shell_tools.rs` around lines 420–475 and 660. The generic shell uses a restricted PATH, HOME `/tmp/ag1-home`, and fresh tmpfs `/tmp` per call. Rustup shims therefore do not see the normal toolchain configuration. The separate native project runner already resolves configured toolchain paths, demonstrating inconsistent execution environments.

**Proposed correction:** share validated toolchain resolution across native checks, generic shell and jobs. Make required executables and their runtime/configuration dependencies available without granting arbitrary writes outside the project. Treat configured toolchains as host capabilities, not permission to execute installers. Define cache behavior deliberately: a persistent project-scoped cache or explicitly safe existing cache, rather than accidental ephemeral downloads on every command. Do not globally replace sandbox HOME with the real home or disable workspace policy.

**Later tests:** ordinary cargo build works with Rust installed outside `/usr/bin`; real binaries and rustup-shim configurations; paths with spaces; shell and job parity; missing-toolchain diagnostic is honest; no repeated downloads caused by ephemeral HOME; install attempts remain rejected; project-write and outside-workspace protections remain intact. Include platform-specific resolution on Windows/macOS rather than hardcoding this machine's `/home/alex` paths.

### C. Resolve certificate bundles inside the native Linux sandbox

**Observed:** UI Runtime target discovery found Cargo.toml. The native Rust check launched but failed fetching serde metadata with SSL error 60, “unable to get local issuer certificate (20).” The successful shell build is a separate operation and does not establish that the UI check works.

**Source:** `product/crates/ag1-platform/src/projects/runtime.rs` around lines 649–664 and 710–744. The sandbox mounts `/etc/ssl`, but on this Arch installation its certificate files link into `/etc/ca-certificates/extracted/tls-ca-bundle.pem`, outside the mounted tree. Native check uses its own initially empty `target/ag1-cargo` cache.

**Proposed correction:** discover and include the resolved trusted CA bundle/dependencies read-only, using bounded canonicalization and platform-aware logic. Preserve TLS verification. Do not solve this by disabling certificate checking or assuming all Linux distributions have Arch's layout.

**Later tests:** fixtures for direct CA files and external symlink targets; missing or invalid trust store gives a clear diagnostic; valid TLS fetch from an empty project cache; certificate failure still rejects the endpoint. No external network is necessary for most regression coverage: use an appropriate controlled TLS fixture.

### D. Require an explicit edit-preview replacement

**Observed:** nine omitted replacements became valid deletion previews. An explicit JSON example finally caused the model to supply `newText`; preview and apply then changed one real comment correctly.

**Source:** `product/crates/ag1-core/resources/tool-catalog.json` around lines 123–140 requires only `path` and `oldText`, and defaults `newText` to an empty string. Native `file_tools.rs` also defaults missing `newText` to empty. The actual offline Rust provider converter preserved this schema. The captured llama.cpp template renders the full tool object. Retained provider argument text already omitted the field: there is no evidence AG1's converter stripped a supplied replacement.

**Proposed correction:** require `newText` in the schema and adapter validation. Explicit empty string remains a valid intentional deletion. Missing, null, and invalid-type values should fail before any effect with a precise correction. Keep exact-match and atomic batch semantics; do not enable fuzzy matching by default to conceal model mistakes. Explain clearly that `editApply` takes `path` plus ordered `edits`, not a preview ID, and that a preview is neither write authorization nor mandatory for every ordinary edit.

**Later tests:** missing/null replacement rejected; explicit deletion works; normal multiline replacement works; Unicode/quotes/CRLF; ambiguous and overlapping matches rejected; failed batches leave bytes untouched; provider-visible schemas match the native contract; real native session journals errors correctly. The successful live apply call is `XMNr9s3mPKts8l4b2jzBVoOI79AYoeAa`, preceded by preview `I0FlwG290BLMCBSfjQ9tm91XB77WrOx2`.

### E. Give delegated work a realistic budget and truthful outcome

**Observed:** the first `task` ended at exactly 60 seconds. A narrower read-only task completed in 26 seconds. Two three-question `explore` calls delivered no findings; all workers failed, but both wrappers returned `isError=false`. The local endpoint had one slot, so multiple workers shared a serialized inference resource.

**Source:** common deadline creation in `session/files.rs`; delegated execution in `product/crates/ag1-platform/src/session/subagent_tool/`; `explore.rs` around lines 187–192 constructs the wrapper result without an all-workers-failed flag.

**Proposed correction:** coordinate outer orchestration budgets with child limits and backend concurrency. Preserve bounded execution, user cancellation, checkpoint/resume, and cleanup. Distinguish queued, running, partially completed, failed, and cancelled work. Return failure when all workers fail; expose partial status and useful findings when some succeed. Avoid launching apparent parallelism that silently spends most of a short budget waiting for the same local slot. Do not fix this by allowing unbounded children.

**Later tests:** all-success, mixed, all-failed and user-cancelled groups; queued single-slot local work; multi-slot/API work; parent timeout propagation; no orphan processes; resumed checkpoint does not duplicate effects; UI and model see worker progress and failure counts.

## 5. Context and progress repairs

### F. Keep mutable memory out of the stable prompt prefix

**Observed:** request 72 had a 7,063-character system prompt and 40,772 cached tokens. After memory `m1`, request 73 had a 7,481-character system prompt, a new memory revision, zero reported cached tokens and 34,617 fresh input tokens. Prefill took about 98.9 seconds. Weights did not reload.

**Source:** `product/crates/ag1-platform/src/session/context.rs` around lines 185–199 appends selected memory to the system prompt through TurnAddenda. The working-set reuse key also includes memory revision. Existing request-only `append_working_state` provides a tail mechanism distinct from durable history.

**Proposed design to evaluate:** keep stable policy and repository verification requirements in the system prompt; project mutable memory as explicitly labelled context/evidence at the request tail. Preserve freshness labels, source validation, loaded-memory identity and admission witnesses. Instructions inside retrieved memory remain data. Separate memory projection from compiler-history reuse only where correctness permits it; do not remove memory revision from admission checks. Include the tail in token budgeting and retained request evidence. Do not accumulate tails in durable history or reusable compiled messages.

**Later tests:** extend `product/crates/ag1-platform/tests/session_working_set_contract.rs`; memory mutation changes the tail while system/tools/eligible compiled-history prefix stay stable; source changes invalidate in-flight admission; no cross-session/branch leakage; real provider conversion and token accounting. Preserve delegation/callback cache-bypass behavior.

**Limit:** the llama server logged some partial prefix similarity but zero reused tokens. The logs do not prove why that partial prefix could not be reused. Dynamic controlled-runtime guidance also enters the system prompt and needs separate policy-sensitive analysis before changing it.

### G. Investigate repeated context reductions without promising universal cache hits

**Observed:** around request 22 the estimated raw context crossed the compiler's approximately 48,000-token target, and four historical write payloads were externalized. Cache reuse fell to roughly 10,796 tokens. Further reductions caused additional expensive prefill. This also occurred without new operator steering.

**Source:** `session/context.rs`, `session/working_set.rs`, and `product/crates/ag1-core/src/context/compiler.rs` around lines 141–166 and 694–854.

**Next work:** retain safe diagnostics for the actual final compiled provider request, not only precompiler input, so the first differing message can be identified. Review reduction granularity and stability: repeatedly rewriting early evidence can erase cache value. Preserve bounded context, exact artifact access and protected evidence; do not simply disable compaction or fill the full 90k context. Evaluate a stable reduction strategy with measured before/after data later.

**Later tests:** repeated long tool rounds crossing the target, no-change rounds, memory updates, steering, protected ranges and artifact rehydration. Record fresh/cached tokens and time to first output separately from weight loading and decode throughput.

### H. Report the current request phase, not the previous phase

**Observed:** “Running tools” remained visible throughout long next-request prefill and while the model generated large tool arguments before execution.

**Source:** `product/crates/ag1-platform/src/client/observations.rs` around lines 33–108 sets phase to `provider stopped` on each provider stop and does not reset it before the next blocking request. `product/apps/desktop/src/App.tsx` around line 78 maps that stale phase to “Running tools.”

**Proposed correction:** emit preparation/admission progress for every request, including later tool rounds, before waiting for provider output. Distinguish request preparation, waiting/processing, thinking, generating an operation, approval, and actual execution. A request-admitted event does not prove GPU prefill has started; label only what is known. Preserve streamed text/thinking, cursor ordering, scope isolation and final receipts.

**Later tests:** provider stop → tool result → next request admitted → long pause → first block; no stale tool label or replayed text; cancellation, reconnect, chat switch and simultaneous API sessions; image-specific processing indicator must clear on completion/error/cancel when tested later.

## 6. UI repairs

### I. Restore file editor structure and responsive sizing

**Observed:** at a 2476×1356 window, Monaco occupied roughly a 240×250 box instead of the available editor area; toolbar labels ran together. See [editor screenshot](../../../../.local/p14-live-project/37-editor-open.png) and [files screenshot](../../../../.local/p14-live-project/33-project-files.png).

**Source:** `App.tsx` around line 257 uses `.editor-panel` and an unclassed header, while `app.css` around lines 587/594 styles `.file-pane` and `.file-toolbar`. The editor lacks the intended flex parent and remains at its minimum height.

**Proposed correction:** align markup and scoped structural classes; provide correct flex/grid sizing, `min-width: 0`, `min-height: 0`, bounded filename truncation and spaced toolbar actions. Ensure Monaco receives resize notifications and uses the available area. Avoid global style patches that distort dialogs or chat.

**Later tests:** large and small viewports, resizing, sidebar/panel toggles, long filenames, scaling, keyboard access and unsaved edits. Browser layout testing is required; jsdom cannot establish Monaco dimensions.

### J. Refresh visible file lists without overwriting work

**Observed:** model-created files appeared only after leaving and reopening Files.

**Source:** `App.tsx` around line 229 fetches files when navigating into Files.

**Proposed correction:** explicit refresh plus bounded refresh after relevant file changes while the panel is visible. Guard project/scope changes and out-of-order responses; preserve selection where possible. Never replace unsaved editor contents with a background refresh. Avoid rescanning the whole project on every token or tool event.

**Later tests:** create/rename/delete while Files is open; rapid project switch; concurrent refreshes; unsaved file; large tree; ignored build directories.

### K. Render previews and tool results as readable, bounded evidence

**Observed:** edit previews displayed JSON with escaped newline strings rather than readable diffs. Compiler output followed a collapsed “Arguments” control without an “Output” heading, making it look mislabeled. Long command labels wrapped into narrow stacks. See [preview](../../../../.local/p14-live-project/57-preview-content.png), [compiler output](../../../../.local/p14-live-project/52-compiler-render.png), and [later compiler output](../../../../.local/p14-live-project/109-next.png).

**Source:** `product/apps/desktop/src/ToolActivity.tsx` and relevant styles. Current generic text rendering treats the native preview JSON literally.

**Proposed correction:** recognize the actual native edit-preview result and decode its hunks into a clear unified-diff view, with a raw fallback for malformed/unknown payloads. Label Arguments, Output and Preview explicitly. Keep call/result pairing by stable call ID. Keep operation labels on one line and let targets truncate within flexible space; provide full command access. Offer copy/wrap/expand controls consistently. Render source/HTML literally, never as executable markup.

Preserve lazy loading, bounded initial output and artifact paging. Do not eagerly parse or mount every collapsed multi-megabyte result. Compiler errors should be easy to find without concealing warnings or full logs. A success icon must describe the actual operation outcome, not merely receipt of a result.

**Later tests:** multiline diffs, explicit deletion, quotes, Unicode, malformed JSON, literal HTML, huge results, copy fidelity, horizontal scrolling/wrapping, paired results, keyboard expansion, collapsed rendering costs.

### L. Make approvals understandable and truthful

**Observed:** dialogs displayed raw JSON or a file path instead of the actual proposed change. The close icon did nothing. Dialogs appeared while typing/steering, and an expired request had no useful countdown/recovery explanation. Changing the approval picker during generation could update its label without changing the active runner's policy.

**Source:** approval UI in `App.tsx`; `applyApproval`/`ensureChat` around lines 160–191; native `product/crates/ag1-platform/src/client/approval_policy.rs` around lines 73–85 requires settled runtime/no queued work.

**Proposed correction:** show human-readable operation, scope and diff/command with optional technical details. Make close behavior explicit—dismiss, deny or disabled with an explanation—and ensure keyboard input cannot accidentally approve. Display the effective mode separately from a pending next-response selection, or disable changes with a clear reason. Do not show a mode as active before native acceptance. Preserve the previously requested second confirmation for bypass and safe new-chat/restart defaults; those were not comprehensively retested here.

**Later tests:** active/idle/queued mode changes, rejected native changes, accidental Enter, focus restoration, expiry/review timing, bypass cancellation/acceptance, restart and new-chat policy.

## 7. Tool contracts, evidence and recovery

### M. Todo query must return the current normalized checklist

The model repeatedly used conflicting clear/new-plan combinations, changed completed identities or omitted required lifecycle context. Those are invalid requests, but recovery is unnecessarily opaque. `product/crates/ag1-core/src/tools/todo.rs` around line 119 advertises a no-argument query that returns generic “No change” with `details.todos=null`. Auto-assigned IDs are not easy to recover after context reduction. `src/todo.rs` around lines 67–81 enforces identity correctly.

Return the current normalized list and IDs on query; identify the conflicting item and allowed recovery in errors. Preserve completed-item identity and explicit archive/new-plan rules. Do not relax them just to make invalid mutations pass. Test no-argument query, lost-context recovery, partial status updates, completed items and conflicting flags.

### N. Keep recorded claims distinct from verified evidence

Memory `m1` attributed a formatted read-output hash to a source file and omitted the canonical `file:` prefix. It was classified unverified. `runtimeObservation` cited an existing failed-test artifact while claiming a successful build; a different build had in fact passed, but that citation did not prove it. An episode initially refused completion with invalid evidence sources and correctly accepted a later partial closure.

Sources: `product/crates/ag1-core/src/evidence.rs` around lines 40–76, 142–160 and 237–321; `product/crates/ag1-platform/src/memory_tools.rs` around lines 121–136. `RuntimeObservation::verified_for` already rejects agent-authored reports as independent proof; preserve that protection.

Improve source-reference ergonomics and diagnostics. Return exact reusable references/digests for the relevant object and distinguish file revision, formatted output artifact and check receipt. UI labels should say recorded/unverified when appropriate. Test stale, mismatched, nonexistent and correctly resolved evidence; an accepted record must never become an independent pass automatically.

### O. Stop coding research from becoming current-events instructions

The query `Rust std io ErrorKind file handling best practices 2024` was classified as current events because a four-digit year matched before code lookup, without whitelist documentation/API words. Fetching official Rust docs inherited that intent and injected harness-written guidance such as “Answer from it now” and current-events coverage slots.

Sources: `product/crates/ag1-core/src/tools/research.rs` around lines 127–151, 574–615 and 682–690; `product/crates/ag1-platform/src/web_tools.rs` around lines 136–166.

Narrow classification and remove generic action-directing advice that overrides the coding task. Search/fetch should provide evidence and relevant navigation, not tell the agent to prematurely finish. Test programming/version/year queries, real current-events queries, explicit docs intent and inherited URL hints. Preserve source provenance and untrusted-content boundaries.

### P. Explain browser prerequisites and support the intended project workflow

`screenshot_url` twice and `interact_then_screenshot` once failed with only “Tool target was not found.” This was not a model-vision failure. A ready AG1-owned web-preview grant was absent.

Sources: `product/crates/ag1-platform/src/client/projects.rs` around lines 119–180; `src/projects/runtime.rs` around lines 1266–1289; `product/crates/ag1-core/src/tools/targets.rs` around lines 40–68. Current discovery recognizes a root package.json dev script whose first token is `vite`; native run allocates the loopback port and verifies package identity. The editor's srcDoc Preview and an arbitrary bash/job server do not create that grant.

At minimum return the missing prerequisite and exact supported recovery path. Decide explicitly whether generic static HTML preview is a supported product feature and implement it through owned/validated targets if so. Do not bypass target ownership or let arbitrary URLs inherit project privileges. Test discovery, startup, stale grant, port collision, wrong identity, script/URL rejection and unavailable runtime. Later exercise screenshot, interaction and caller-vision verification on a valid owned target.

### Q. Correct submit_plan's policy classification/message

The ordinary desktop adapter is configured with planning support in `client/tooling.rs` around line 43, so the adapter exists. Do not repeat the abandoned diagnosis that it was absent solely because `adapters.rs` contains a planning guard. In the live interactive session the policy fallback described submit_plan as “not a built-in AG1 tool” and requested generic approval. The request eventually expired.

Source: `product/crates/ag1-core/src/tools/policy.rs` around lines 504–513, plus adapter/catalog setup. Audit intended applicability, builtin origin and policy classification together. A legitimate builtin plan submission should not receive a misleading unknown-tool message. Unknown extensions must not acquire builtin privileges by name. Test ordinary/planning modes, malformed plan arguments, origin checks and approval behavior. The model's proposed repair plan also contained stale project facts; that is separate from this policy issue.

### R. Bounded recovery rather than repetitive broad reads and retries

The model repeatedly retried stale edits, redundant checklist updates and old compiler diagnoses. Direct edits already correctly reject mismatches/no-ops; keep those checks. Improve concise recoverable errors and make current source snippets/IDs accessible. Review repetition handling against this transcript, especially semantically identical retries whose narration changes.

The operator initially gave two incorrect pieces of guidance (a todo completed-boolean assumption and a preview-ID assumption) and corrected them. No preview-ID apply was attempted. Do not attribute every detour solely to the model or harness. Future tests should use precise contracts and preserve the operator intervention log.

Do not resolve loops by silently reporting success, dropping requested work, weakening exact matching, or adding a huge system prompt. Test bounded recovery from one stale edit, unavailable environment, invalid checklist and failed child task.

## 8. Things that worked and must not regress

- Project attachment accepted the new folder and created a separate project entry.
- Empty drafts did not create new sidebar conversation rows during the observed navigation.
- New chat could open while the original chat generated; its spinner continued and returning restored the correct active conversation.
- The first user message remained reachable while generation continued. Reading position held; jump-to-live was available.
- Thinking started collapsed and could be expanded.
- The 35B picker exposed a binary Thinking On control consistent with the observed capability.
- Settings opened during generation; Graphite → Midnight → Graphite applied immediately and returning to chat worked.
- File writes and many direct edits produced real durable changes. Exact mismatch/atomicity protections worked in observed cases.
- Installer attempts were blocked. No compiler/SDK installation was executed.
- A narrow delegated review found a real empty-file behavior issue. Background job start/poll worked after environment correction.

Opening thinking close to the live bottom shifted its header upward; review anchor behavior later. These observations are not comprehensive proofs of all chat/concurrency/theme features.

## 9. Coverage gaps and unfinished project work

Do not say “all tools passed.” Twenty-six of the original 28 tool names were attempted. `append` and `verify_ui` remained unattempted. `check` appeared in a later expanded catalog but was not called as a model tool; the UI check was separately attempted and failed TLS acquisition. `artifact` coverage was a lease, not meaningful artifact read/grep/navigation. `diff` on a non-Git project and `find` with unhelpful patterns did not establish useful positive coverage.

The Trailbook README and TOOL-COVERAGE.md are model-authored outputs, not the audit oracle. Known inaccuracies: README suggests `cargo test --offline --lib` in a binary-only project, describes a Linux data directory inconsistent with current code, and preserves an obsolete `into_owned()` diagnosis after the edit. Integration test isolation and empty-file behavior need verification. Do not silently correct the fixture before preserving it; those mistakes are valuable regression inputs.

Still unverified here: image paste/thumbnail/full preview/analyzing indicator; actual 165 Hz frame delivery; multi-provider concurrent thread behavior; Windows/macOS parity; fresh restart approval defaults; bypass second confirmation; plugin/theme import/export; model switching across distinct models; GPU saturation as a performance target. Prior user requests remain relevant, but this run does not establish their status.

## 10. Suggested future implementation and test order

This is a proposed sequence for a later authorized session, not work performed now.

1. Preserve the installed release, dirty source baseline, native transcript and model profile. Reproduce deterministic contracts with controlled providers/clocks before GPU work.
2. Fix approval phase budgets, required preview replacement, toolchain resolution and CA access. These directly blocked ordinary coding.
3. Fix delegation budgets/status and truthful progress phases. Repair todo query and evidence-reference diagnostics.
4. Address mutable-memory placement and instrument compiled-prefix changes. Keep context bounded; measure rather than promise cache reuse.
5. Restore editor layout, visible file refresh, readable diff/output and effective approval-state display.
6. Correct research guidance and builtin-plan policy messages; clarify browser preview setup.
7. Run focused native/frontend regressions and typecheck after implementation. Build only with enough disk and the correct target directory. Do not treat historical test receipts as validation of new changes.
8. When separately authorized, perform a short live UI retest using the exact 35B model and a fresh isolated project. Do not start the eight-hour run implicitly.
9. Only after evidence review should a later session build/install a reversible release, if authorized. Keep rollback identity and commands explicit.

### Later live acceptance script

- Verify model identity from backend and process configuration, not the model's self-description. Record model path, server capabilities, loaded profile and app/release identity.
- New chat with the warm model; create a small Rust project through normal UI. Ordinary cargo must work without operator PATH coaching. Check/build/test outcomes must be separate.
- Delay one approval beyond 60 seconds, approve it, and verify full execution budget; separately deny, expire and cancel with no file effects.
- Ask for two small genuine edits. Inspect readable previews, approve, verify diff and tests. Deliberately stale one oldText and verify one useful recovery rather than a retry loop.
- Start a bounded review and exploration; inspect child progress, mixed/all-failed outcomes and cancellation. Exercise one background job with the same environment.
- Add memory during a multi-round session; record actual request-prefix differences, fresh/cached tokens and latency. Cross the reduction threshold in a separate bounded case.
- Switch chats and settings while work runs; verify session isolation, history access, anchors, live text/thinking and truthful progress.
- Open Files at large/small sizes; create a file while open; preserve an unsaved edit during refresh. Inspect diff, compiler output and literal HTML safely.
- Start a supported owned web preview, then screenshot, interact and verify it. Exercise image paste and full-resolution previews before/after send; observe analyzing state through success/error/cancel.
- Exercise append and meaningful artifact navigation; use a Git-backed fixture for a real change diff. Finish with actual checks and an evidence-backed summary.
- Measure frame intervals and long tasks during scroll/stream/expand on a 165 Hz display (about 6.06 ms per refresh), then separately on Windows/macOS. Monitor refresh alone is not proof, and fixed-rate timers are not an adequate solution.

## 11. Evidence, release identity and preservation

Primary evidence:

- [Frozen observer report](../../../../.local/p14-live-project/live-audit-report.md).
- [Supplementary runtime design handoff](../../../../.local/p14-live-project/runtime-fix-handoff.md).
- [Exact failure attribution and call IDs](../../../../.local/p14-live-project/failure-attribution.json).
- [Machine-readable audit](../../../../.local/p14-live-project/audit.json).
- [UI observations](../../../../.local/p14-live-project/ui-observations.json).
- [Provider schema investigation](../../../../.local/p14-live-project/wire-schema-audit.json).
- [Run identity](../../../../.local/p14-live-project/run.json) and [ordinary UI action log](../../../../.local/p14-live-project/actions.jsonl).
- Numbered screenshots and operator prompts in `.local/p14-live-project/`.

Native store: `/home/alex/.local/state/ag1/desktop/native/1c1524deba4073c08c6a935d7a4122bf7fd4064fe5ab735ee253bf3ba8c04a35/store/storage.sqlite`. Sessions include `initial` and delegated children. Read it read-only. Owned runtime logs are in the adjacent `owned-llama-runtime/` directory. The app log is `.local/p14-live-project/desktop.log`.

Installed tested release: `/opt/ag1/releases/2ebdc491fb814b959b79378f24c62e01`, launched through `/usr/local/bin/ag1-desktop`.

Recorded source identity: `10c08fa628ca8ab43cd9257aef00d81dc8c5de4346fafe20767e2091543ec30a`. Package `.local/p14-packages/52e239d35698412daa640fb237c8ded0/ag1-0.1.0-rc-linux-x86_64.tar.gz`, SHA-256 `e1be8dba662bd42e246d28d775307b9c4cd2f1efac9e453e17a4cb7561dd0032`.

Earlier edit-reliability corrections and tests are documented in [the previous receipt](../../P14/desktop-edit-reliability.md). They predate this live test and do not cover these newly discovered defects. Do not erase those fixes while implementing the next ones.

The repository was already heavily dirty. Pre-repair snapshot: `.local/p14-live-fixes/before-source.tar.gz` (575 existing changed/untracked files), `before-paths.json`, `before.patch`, and `started.json`. It is a preservation aid, **not** an instruction to restore all files blindly. The snapshot is not a full repository clone and does not itself delete files added afterward. Preserve unrelated changes and restore only explicitly reviewed paths if needed.

At the end of the test `/home` had about 2.5 GB free. Do not delete user files or model weights to make room. Inspect disk usage and use targeted, reviewable cleanup of owned regenerable artifacts only when later authorized. No broad clean/build was performed in the cancelled repair phase.

Future Rust work should use `CARGO_TARGET_DIR=/home/alex/Projects/AG1/.local/p14-target` and `CARGO_INCREMENTAL=0`, subject to checking disk first. **Do not use or rebuild `product/target` casually**: the existing bound process host and accepted capture are preserved inputs.

Preserved identities from the prior release work:

- `~/.ag1/agent/llamacpp-model-profiles.json`: SHA-256 `aa02688c4d39ef73234f0ec381eed22dbe02463881626423049b75b5a12ad0d0`.
- `product/target/release/ag1-process-host`: SHA-256 `e1c7ca1711b2b7834eea1c99a55fe958cb4cfb880ed3c6909b06232c36bc1da2`.
- `.local/p13-runtime-capture-20260907/configuration.json`: SHA-256 `b6ea6f521e34c0278dd5d3c097ca2c83036eeebd52cb49305c4623734f3a9fba`.

Source line numbers and runtime PIDs are historical anchors. Confirm current source and process ownership before acting. The `.local` evidence is machine-local and may not be versioned; preserve or deliberately package it before moving this handoff to another machine. It can contain project paths and transcript content, so do not publish the raw bundle inadvertently.
