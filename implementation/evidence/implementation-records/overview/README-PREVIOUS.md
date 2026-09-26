# Implementation status and records

> Historical evidence, archived 2026-09-13. Status, commands, permissions and paths below describe their original checkpoint. For current work start with [implementation status](../../../README.md). Original bytes and old paths are preserved in the [relocation manifest](../manifest.json) and recovery archive.

Latest desktop repair: [monitor refresh-rate clock](../desktop/DISPLAY-REFRESH-20260913.md), installed systemwide. NVIDIA/WebKit fallback now uses native Wayland GTK frame timing: controlled probe 165 FPS; actual AG1 Settings scroll median 6 ms. Restart AG1.

Latest MCP repair: [pagination and Discover loading](../mcp/MCP-DISCOVER-LOADING-20260913.md), installed systemwide. Bounded listing summaries fix oversized page failures; retained public display data and concurrent icon fetching improve reopening. 27 focused checks passed. Restart AG1.

Latest MCP UI: [additional readability pass](../mcp/MCP-DISCOVER-READABILITY-20260913.md), installed systemwide. 52 px icons, 19 px names, 17 px descriptions, 15 px metadata and slightly more row padding. Restart AG1.

Latest MCP sizing: [larger icons and text](../mcp/MCP-DISCOVER-SCALE-20260913.md), installed systemwide. Icons are 48 px; names 18 px; descriptions 16 px. Restart AG1.

Latest MCP UI delivery: [readable rows and persistent details pane](../mcp/MCP-DISCOVER-ROWS-20260913.md), installed systemwide with both launchers using the new build. Larger icons/text, metadata beneath descriptions, adjacent Add actions and a 62/38 wide-screen directory/details split. Restart AG1.

Latest MCP UI fix: [publisher icons and row readability](../mcp/MCP-DISCOVER-ICONS-20260913.md), installed systemwide and per-user. Real icons, cleaner display names, larger descriptions and aligned version/publisher/connection/action columns. Restart AG1.

Latest MCP UI delivery: [Discover desktop redesign](../mcp/MCP-DISCOVER-UI-20260913.md), installed for this user. Full-width directory rows, local filtering, initial public listings, category/recency sections and responsive details reuse Settings styling. Native MCP/security behavior is preserved. Restart AG1. The same build is also installed systemwide.

Latest MCP delivery: [core integration acceptance](../mcp/MCP-INTEGRATION-ACCEPTANCE.md), complete and installed on 2026-09-13. Native management, authentication/scoped trust, public discovery/guided setup, shared model execution, bounded input and media are integrated. Final selected Rust checks: 238 passes; frontend: 243 passes and two unchanged known failures. Production startup and 1,144 installed payload files passed. Restart AG1. Optional compatibility limits and separate paused work remain documented.

Latest UI change: [hide retained-output footer](../desktop/HIDE-TURN-OUTPUTS-20260911.md), installed and verified. The bottom retained-outputs card is temporarily hidden; Artifacts remains available.

Latest desktop repair: [project folder picker](../desktop/PROJECT-FOLDER-PICKER-20260911.md), installed and verified. Projects + opens native folder browsing with folder creation enabled. 32 App tests passed. Weekly usage: 0% remaining.

Latest skill repair: [native skill activation/loading](../desktop/SKILL-ACTIVATION-REPAIR-20260911.md), installed and verified. Automatic triggers and explicit Use now embed captured instructions; native attachment delivery is tested.

Latest SVG repair: [specific XML diagnostics](../desktop/SVG-PARSE-REPAIR-20260911.md), installed and verified. The reported complexity failure was an unmatched closing tag; the detailed artwork renders after that syntax correction.

Latest image repair: [direct local image preview](../desktop/IMAGE-PREVIEW-ROUTE-20260911.md), installed and verified. `ui({"path":"pelican-bike.svg"})` returns a visible image without a server; blur/shadow filters supported. 11 tests and the actual pelican/Qwen vision payload probe passed.

Latest UI repair: [startup/loading progress](../desktop/STARTUP-PROGRESS-20260911.md), installed and verified with 45 tests and two isolated browser widths. Active status and elapsed time appear before the first assistant/tool output.

Latest control repair: [local thinking before sending](../desktop/LOCAL-THINKING-RESTORE-20260911.md), installed and verified. Qwen 3.6 has Off/On; both configured 3.8 models have Off/Low/Medium/Extra high, available before loading.

Latest stability repair: [Luna stream failure and cancellation](../desktop/LUNA-STREAM-REPAIR-20260911.md), installed and verified with 76 tests, native failure/cancellation recovery, and a real read-only Luna/GitHub call. The frozen previous instance was closed at the user’s request.

Latest UI delivery: [compact response workflow](../desktop/WORKFLOW-UI-20260911.md), installed and verified with 50 tests and nine isolated visual states.

Latest UI delivery: [Markdown readability](../desktop/MARKDOWN-READABILITY-20260911.md), installed and verified with 15 focused tests and three isolated browser layout/clipboard checks.

Latest delivered change: [model interface consolidation](../desktop/INTERFACE-CONSOLIDATION-20260911.md), installed and verified with 23 focused tests, a packaged native workflow and a bounded real Luna check. New chats receive a smaller default tool surface; existing explicit selections remain intact. Prior checkpoints below are historical.

Start with the [remaining task index](../../../../tasks/README.md). It contains the current split task files and execution boundary. A previous delivered checkpoint is the [Astra deeper interface fixes](../desktop/ASTRA-DEEPER-INTERFACE-20260911.md), installed and live verified with Luna and Qwen35B. Earlier [GitHub usability](../github/GITHUB-USABILITY.md) and [OpenAI subscription models](../desktop/OPENAI-OAUTH-MODELS.md) remain completed checkpoints. Artifact/review implementation history is consolidated in the [checkpoint record](../desktop/P14-ARTIFACT-REVIEW-CHECKPOINT.md).

The [release resume record](../phases/P14-RESUME-HANDOFF.md) retains unfinished broader P14 release work and partial-run custody. Its old process/candidate details require reconciliation before execution. No feature or release work is started by the documentation reorganization.

## Historical phase-by-phase implementation notes

The following entries describe their original phase checkpoints. Old next-phase and runtime-authority statements must not override current completion evidence, installed configuration or the task index.

P10 delivers the native session/runtime facade in isolated workspaces. Its authoritative status, exact commands, hashes, trace coverage and later gates are in [completion evidence](../../P10/completion.json), with the [report](../../P10/REPORT.md) and [session guide](../../../../product/crates/ag1-platform/SESSION.md). The normal desktop remains TypeScript authority and all tuned llama.cpp runtimes/settings remain preserved. Earlier P10 checkpoints are archived in [history](../../P10/HISTORY.md).

P11 delivers concrete web/native/Rust/Tauri/Godot project targets, owned lifecycle/logs, constrained previews, native tool integration and guarded Git recovery. Its [completion evidence](../../P11/completion.json), [report](../../P11/REPORT.md) and [project API guide](../../../../product/crates/ag1-platform/PROJECTS.md) record acceptance and limitations. The next thread starts with [P12-HANDOFF.md](../phases/P12-HANDOFF.md). The original [P11 entry handoff](../phases/P11-HANDOFF.md) remains a historical checkpoint.

Implementation began with P01 on 2026-09-05 (Europe/Oslo). Follow `ROADMAP.md` and `phases/README.md`; the machine authority for completion is each phase's `completion.json`, including its tests, artifact hashes and pending gates.

P01 implements the isolated reference/bootstrap, runtime fingerprinting, executable scenario catalog, launch/request/session captures, frontend provenance inputs and fail-closed validation tools. Python and JavaScript here are development/reference harnesses. The final product core remains Rust with Tauri and React.

The expanded reference suite revealed a stale cross-turn prefix test. The current source intentionally attaches working-state projection to the latest user message, preserving canonical history and stability within a tool loop. Its exact original failure and a passing independent replacement capture are recorded in `evidence/P01/REFERENCE_DIVERGENCES.md`; it is not silently counted as passed. Credential-dependent and Windows-only tests remain explicit later gates.

Original AG1, user profiles, GGUFs and all three tuned llama.cpp builds remain untouched. No product runtime or migration authority is transferred by P01.

P02 adds the compiling Rust core/protocol workspace, generated JSON Schema and TypeScript, and a deterministic fake host. Its gate runs contract tests, Rust/TS/schema round trips, parser/admission/order checks, formatting, Clippy, licenses and runtime preservation checks. See `evidence/P02/completion.json` for its actual completion status. The generated frontend types do not constitute a desktop application. P03 supplies platform adapters next; P04 introduces the Tauri/React desktop and guarded original TypeScript worker.

P03 implements Linux workspace/filesystem/Git foundations, byte snapshots and atomic revision-checked mutations, bounded indexing/search, an owned child-subreaper helper, process/PTY lifecycle and a separate worker stdout transport. HTTP/secret ports have explicit fakes. The gate exercises disposable workspaces and controlled process trees, rechecks P02 contracts and records small/large fixture timings. `evidence/P03/completion.json` is authoritative for completion; native Windows/macOS tests remain pending. Tuned llama.cpp ownership and settings remain unchanged.

P04 implements the React/Tauri desktop, supervised original SDK bridge, durable command ledger, bounded projections and native file/approval flows. It records corrected B01–B04/D01 behavior, matched native visual evidence, five-minute process measurements and actual renderer/license closure. See [the P04 report](../../P04/REPORT.md), [completion authority](../../P04/completion.json) and [development run instructions](../../../../product/apps/desktop/README.md). The TS SDK remains authoritative; live inference, other OS runners, interactive preview and final Rust parity remain later gates.

P05 adds independent Rust SQLite/blob storage, restartable frozen imports, lossless export/restore, authenticated evidence closure across forks, corruption retention and command reconciliation. See [the storage guide](../../../../product/crates/ag1-storage/README.md), [P05 report](../../P05/REPORT.md) and [completion authority](../../P05/completion.json) for actual results and remaining gates. The current desktop continues to use its TypeScript worker; no live journals or model runtimes migrate in P05.

P06 passed its executed gate: 88 Rust contracts, eleven gate-validator tests, all ten pinned provider families, exact preserved launch/request captures and matching synthetic benchmark output. See [the provider guide](../../../../product/crates/ag1-platform/PROVIDERS.md), [P06 report](../../P06/REPORT.md) and [completion evidence](../../P06/completion.json). The existing llama.cpp materials were verified unchanged; live provider/model and other-platform checks remain pending.

P07 passed its executed gate: 87 Rust contracts, seven gate-validator checks, 94 unchanged SDK state cases and matched-input source/state measurements. See [the state API guide](../../../../product/crates/ag1-core/STATE.md), [P07 report](../../P07/REPORT.md) and [completion evidence](../../P07/completion.json). Forced source races reject mixed text/hash results, memory edits use durable CAS and task/evidence records survive restart. P08 retrieval and context compilation follows with TS authority preserved.

P08 passed its executed gate: 81 Rust contracts, eleven unchanged SDK probes, seven gate-validator checks and repeated cold/warm/rehydration measurements within the 10% p95 limit. See [the context API guide](../../../../product/crates/ag1-core/CONTEXT.md), [P08 report](../../P08/REPORT.md) and [completion evidence](../../P08/completion.json). Retrieval preserves exact provenance and freshness; episode/summary drafts remain isolated and non-authoritative; reductions wait for durable original custody. TS authority and tuned llama.cpp remain unchanged. P09 tools, policy, resources and extension execution follows.

P09 passed its executed gate: 491 selected Linux cases accounted for (467 covered, 24 documented corrections), two Windows cases explicitly pending, 52 immutable SDK probes, 272 native/helper tests and four native driver tests. See [the tool API guide](../../../../product/crates/ag1-core/TOOLS.md), [compatibility decisions](../../../../product/crates/ag1-platform/TOOL_COMPATIBILITY.md), [P09 report](../../P09/REPORT.md) and [completion evidence](../../P09/completion.json). Tools retain full evidence, bind approvals to current authority and recover without repeated effects; native check completion atomically releases verification leases. Three measured runs verify file concurrency, full shell spooling and bounded owned process observations. Exact dependency notices include the optional loader bundle. The TypeScript desktop and tuned llama.cpp materials remain unchanged. P10 session/runtime integration follows.
