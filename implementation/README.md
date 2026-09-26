# Implementation overview

Start here. Historical handoffs are stored as evidence; the work queue is in [tasks/README.md](../tasks/README.md). This overview was reconciled on 13 September 2026 and does not resume paused implementation or duration testing.

## What still needs doing

| Work | Current status | Start here |
|---|---|---|
| Qwen3.8 Flash-Next optimized tune | Investigation complete; optimized Flash-Next tune subsequently deployed at the user's request. Existing 35B and 27B launch plans preserved exactly. Startup, RAM and recovery tradeoffs remain documented. | [Deployment](evidence/flash-next-cutover-20260915/REPORT.md), [performance report](evidence/flash-next-performance-20260914/REPORT.md), [reproduction guide](evidence/flash-next-performance-20260914/REPRODUCTION.md) |
| Artifact and change-review closeout | Remaining: reconcile current coverage, finish actual presentation gaps, and verify non-file outputs, large diffs, focus/drafts and Undo/recovery. Much of the implementation already exists. | [Task 1](../tasks/01-ARTIFACT-REVIEW.md) |
| Combined chat presentation cleanup | Remaining: review nesting, result/error clarity, accessibility, preserved drafts and history behavior across existing cards and panels. | [Task 3](../tasks/03-CHAT-CLEANUP.md) |
| Combined feature verification | Remaining after the above work. The completed nine-scenario harness exercise covers a narrower scope and does not close this task. | [Task 4](../tasks/04-FINAL-VERIFICATION.md) |
| Broader P14 release | Separate, paused backlog: reconcile remaining model comparisons/quality, packaged performance, other platforms, signing and release gates. Duration work requires a separately resumed scope; no eight-hour run is authorized by these documents. | [Release phase](../phases/P14-release-and-long-horizon-gates.md), [historical custody record](evidence/implementation-records/phases/P14-RESUME-HANDOFF.md) |

No pending overnight-fix implementation is established by the old handoffs. Read newer completion evidence before treating an old checklist as outstanding work. Old candidate paths and runtime observations must be reconciled with the current machine before reuse.

## Latest completed work

- **AG1 activity mark and whimsical verbs:** [delivery and verification](evidence/activity-personality-20260914/REPORT.md). Installed a themed three-stroke mark, text shimmer and 251 distinct activity phrases. The live indicator stays at the bottom; critical waiting/loading/approval states remain literal. 284 frontend tests and private motion/theme checks passed. A [development studio and tuning guide](../product/apps/desktop/ACTIVITY-DESIGN.md) make future animation changes independent of the model/backend.

- **Website icon reliability:** [implementation and live comparison](evidence/website-icons-20260914/REPORT.md). Installed declared/CDN icon discovery, safe SVG support, compressed downloads, domain-only fallback, queued fetching and failure recovery. OpenAI's missing logo reproduced with the old fetcher and succeeded with the new one; all six live sites passed. 279 frontend and 13 focused native checks passed. Reopen AG1; llama.cpp and tuning remain unchanged.

- **Full AG1 desktop validation of the optimized 35B backend:** [results and reproduction evidence](evidence/ag1-optimized-validation-20260914/REPORT.md). Real app-owned inference passed chat, exact editing, web tool execution, new-chat reuse and Stop/recovery. The optimized backend was faster in each task in one desktop comparison; both web answers retained factual/instruction errors. Original installation, configuration and model files remain unchanged; no production cutover.

- **Completed overnight performance investigation:** [Qwen3.6-35B-A3B results, source patch and standalone build](evidence/llama-performance-20260914/REPORT.md). About 6× main-review prefill, 18% faster review decode and roughly 3× exact-edit decode; cold-start and tiny-request tradeoffs are documented. Original binaries, AG1 configuration and model files passed the final preservation audit. A [cross-model reproduction guide](evidence/llama-performance-20260914/REPRODUCING-WITH-ANOTHER-MODEL.md) records the reuse procedure and limits. AG1 still uses the original; no systemwide cutover occurred. This does not resume the separate P14 feature/endurance backlog.

- **Manual model loading and progress:** [implementation and checks](evidence/model-loading-20260914/REPORT.md). Installed themed Load/Switch & load control, persistent Loaded status, measured verification percentage and loading-stage spinner. Drafts stay editable; loading shares generation admission. 276 frontend and 14 native checks passed.

- **Markdown table wrapping fixed:** [repair and verification](evidence/table-layout-20260914/REPORT.md). Installed whole-word table wrapping and top-aligned cells; saved-answer checks passed at three widths. Reopen AG1 to load the new renderer.

- **New-chat `NotFound` fixed:** [recovery and real 35B send verification](evidence/chat-launcher-recovery-20260914/REPORT.md). Restored the exact pinned AG1 model supervisor removed by earlier release cleanup. The installed native client now opens and completes a real message using the desktop configuration. Model tuning/configuration and the current application release are unchanged.

- **Free search quality improvements:** [implementation, native checks and bounded 35B findings](evidence/search-quality-20260914/REPORT.md). Installed: rank a larger candidate pool, reuse concurrent searches, prefer relevant project documentation, preserve code/tables and deliver focused source passages. Twelve native searches completed without blocking; 54 focused checks passed. Better evidence delivery is verified; model answer correctness and a separate CLI research-completion issue remain open.

- **Working free DuckDuckGo search:** [implementation, live results and You.com evaluation](evidence/free-search-recovery-20260914/REPORT.md). Installed the no-AI web result flow and bounded arithmetic-check handling. Fifteen native queries completed without blocking (13 with results, two expected empty), median 807 ms; 39 automated checks passed. No fixed search padding or You.com quota dependency. Future DDG changes remain a compatibility risk.

- **DuckDuckGo request flow and rate limits:** [results and installation](evidence/duckduckgo-request-flow-20260914/REPORT.md). Installed POST transport and serialized searches with no fixed success delay; 22 checks passed. Three of seven native probe attempts returned results, but recurring challenges remain unresolved. No published numeric quota was found.

- **DuckDuckGo integration repair:** [implementation and checks](evidence/duckduckgo-fix-20260914/REPORT.md). Installed; 302 automated checks passed. HTML/lite parsing, blocked-search errors, cooldown, explicit queries and suggestion/source separation repaired. Live DuckDuckGo still requires human verification; improved live result quality is unproven. No alternate engine added.

- **Bounded 35B / OpenCode comparison:** [results and screenshots](evidence/bounded-comparison-20260913/REPORT.md). Sequential short runs; AG1 16/18 vs OpenCode 17/18 held-out repair checks, neither fully correct. Favicons and thinking placement worked; research exposed weak extraction and failed-fetch classification. Production unchanged.

- **Thinking placement and website icons:** [implementation and verification report](evidence/chat-activity-20260913/REPORT.md). Source/build complete; 561 final automated checks and two bounded 35B visual exercises. Production package installed systemwide; all 1,144 installed payload files verified. Empty live search responses remain an identified limitation.

- **Fixed harness scenarios:** [validation report](evidence/scenario-validation-20260913/REPORT.md). All nine scenarios passed within stated new/retained coverage; 295 final targeted checks, live cap-7/cap-5 repairs, task/evidence retrieval, and isolated native MCP UI. No new regression or production change; no screenshots or duration run. Cross-model/endurance and native WebKit history performance remain outside that coverage.
- **Overnight harness fixes:** [final implementation record](evidence/implementation-records/overnight/HANDOFF-OVERNIGHT-FINAL-FIXES-20260913.md). Completion/recovery, verifier guidance, work actions, request preparation, retained custody and related fixes were delivered. The scenario test handoff is completed, not a new assignment.
- **MCP:** [core acceptance and limits](evidence/implementation-records/mcp/MCP-INTEGRATION-ACCEPTANCE.md), followed by [Discover loading repair](evidence/implementation-records/mcp/MCP-DISCOVER-LOADING-20260913.md). Core phases A–F are complete; optional compatibility limits remain documented.
- **GitHub and desktop:** [GitHub workflow improvements](evidence/implementation-records/github/GITHUB-IMPROVEMENTS-20260913.md), [capability ledger](evidence/implementation-records/github/P14-GITHUB-CAPABILITIES.md), and [display refresh repair](evidence/implementation-records/desktop/DISPLAY-REFRESH-20260913.md). Earlier interface and UI changes are indexed with their evidence.
- **Storage cleanup:** [cleanup report](evidence/storage-cleanup-20260913/REPORT.md). Historical references can point to retired packages; check availability rather than recreating them automatically.

Latest verified installed release: `/opt/ag1/releases/ff218780597542c58cec2e5c1ba4a025`; see the activity-mark report. Resolve `/opt/ag1/current` before a future build-dependent claim. Documentation reorganization itself did not rebuild, install or restart AG1.

## Finding evidence

- [DuckDuckGo search-engine overview](evidence/search-engines/DUCKDUCKGO.md): what changed, how the working route operates, its effect on results, and proposed next improvements.

- [Evidence index](evidence/README.md): current reports and historical evidence families.
- [Archived implementation records](evidence/implementation-records/README.md): all 76 former top-level Markdown files, grouped by topic and marked historical.
- [Relocation manifest](evidence/implementation-records/manifest.json): old paths, original hashes and new locations. Original documents are also preserved in its linked recovery archive. Existing historical receipts were not rewritten.

`P12-HANDOFF.md` and `P13-HANDOFF.md` remain here only as compatibility symlinks. Release tools and accepted manifests depend on their original paths and exact bytes. Read their indexed archival copies for historical context; neither is a current work assignment.
