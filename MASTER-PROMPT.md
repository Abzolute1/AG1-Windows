# Planning mandate for a fresh Astra instance: AG1 Desktop

You are the planning architect for a new AG1 desktop application. You have no dependency on a prior conversation. Your task is to turn the completed repository research in this workspace into an implementation-grade roadmap and independently executable phase instructions. **Stop after planning. Do not implement the product or execute the phases.**

## Workspace and authoritative sources

- Planning workspace: `/home/alex/Projects/AG1`.
- Vision: `VISION.md` in this workspace. It controls scope and constraints.
- Current AG1 reference: `/home/alex/Projects/Cruzible`, inspected at `e30f0b4b238ee5618928bcd17b4571eb046e7038`.
- Dyad reference: `/home/alex/Projects/AG1/vendor/dyad`, inspected at `fb17039bfd2adcac7d74b9c1178a3a3524df35b6`, branch `research/ag1-desktop`, source `https://github.com/dyad-sh/dyad.git`.
- Optional isolated executable AG1 reference: `research/scratch/ag1-reference`, archived from the pinned revision with copied dependencies and rebuilt existing packages. Scratch is disposable; original repository remains authoritative.

Treat Cruzible as read-only. It had pre-existing deleted proposal documents and untracked demo/proposal files; do not restore, delete or edit them. Research source hashes and status are recorded in `research/evidence`. Do not run its write-enabled formatter/check script or installs/builds there. Future implementation phases should work in the new product/reference copies. Any later exception affecting the original must be explicit and narrowly justified; this planning task grants none.

The Dyad clone is for inspection, not the base of the new product. No remote fork was created because `gh` was unavailable; no remote action is needed to complete planning. Do not add Tauri to Dyad, delete its agent code, or publish a fork during planning.

## Required reading before designing phases

Read `VISION.md` and **all top-level `research/*.md`**:

1. `research/README.md` — source revisions, evidence conventions and limitations.
2. `research/AG1_ARCHITECTURE.md` — actual entrypoints, runtime and ownership.
3. `research/DYAD_ARCHITECTURE.md` — current frontend, IPC, project lifecycle and Electron boundaries.
4. `research/AG1_PARITY_MATRIX.md` — A01–A85 capability ledger, status, paths, invariants, tests and dependencies.
5. `research/DYAD_UI_REUSE_MAP.md` — file/component reuse and minimum AG1 surfaces.
6. `research/LICENSE_AUDIT.md` — controlling license boundaries, unknown assets, transitive exclusions and notices.
7. `research/PERFORMANCE_BASELINE.md` — measured results, workload limits and missing-baseline experiments.
8. `research/BUGS_AND_TECH_DEBT.md` — B01–B04 confirmed defect probes and separately classified risks.
9. `research/INVARIANTS.md` — I01–I30 acceptance properties.
10. `research/TARGET_ARCHITECTURE.md` — proposed ownership/process/protocol/persistence boundaries and migration seams.
11. `research/MIGRATION_RISKS.md` — specific handoff, long-horizon, licensing and platform risks.
12. `research/TEST_STRATEGY.md` — reference tests, fixtures, correction oracles and integration gates.
13. `research/DEPENDENCY_GRAPH.md` — logical work packages, ordering constraints and writer handoffs.

Inspect supporting evidence where relevant: source/test inventories, 465 passing coding-agent tests plus 26 Godot tests, synthetic benchmark samples, CLI metadata startup, source-race/memory/tool-cap and fork-artifact probes, Dyad native/Pro import inventories and dependency-license records. These are **491 tests passed in a selected scope**, not complete feature/provider/platform/live-model parity. No packaged Dyad or complete AG1 desktop performance baseline exists yet.

Verify important paths/functions and any disputed behavior directly against the pinned repositories. If HEAD has moved, compare changes and record the delta; do not silently blend research from two revisions. Use source hashes and execution callers rather than comments alone. Do not repeat the entire discovery unnecessarily. Focus additional inspection on a concrete architectural decision or uncertainty.

## Non-negotiable product direction

- **User requirement, 2026-09-04:** reuse the existing tuned llama.cpp builds and per-model settings. Read `planning/LLAMA_CPP_PRESERVATION.md` and its runtime-lock evidence. Preserve the separate configured backends, libraries, launch configuration and request semantics throughout the bridge and Rust cutover. No automatic upgrade, rebuild, replacement, retuning or fallback; any such change requires separate explicit user authorization. This is an inference-runtime dependency to retain, not agent logic to rewrite in Rust. Carry L01–L08 into the phase/test coverage.
- AG1 is the intelligence, behavior and long-horizon authority.
- Long-term core: Rust, independent of Tauri, React and terminal rendering.
- Desktop boundary: Tauri 2, no Electron dependency in the final product.
- Frontend: React + TypeScript + HTML/CSS, retaining approved Dyad visual language and portable frontend code.
- No “Dyad with AG1 bolted on,” wholesale Dyad dependency tree, or agent logic hidden in React callbacks.
- Preserve intended AG1 behavior, state continuity, provenance/freshness, artifacts, tools/models, recovery, scheduling and native embedding. Fix confirmed defects with tests; do not casually redesign successful semantics.
- Do not add speculative embeddings, reranking, allocator schemes, daemon infrastructure or cloud product features merely because they appear attractive.
- Project/runtime targets must support native applications, including Tauri/Rust and existing Godot-style host integration; a project is not just a preview URL.
- Keep a usable reference implementation until objective parity and regression gates pass. Favor coherent incremental authority transfer over a big-bang rewrite.

## Repository findings you must preserve in the plan

1. The low-level Agent loop is only one layer. `AgentSession`, its coordinators and client modes contain critical context/recovery/policy behavior. Each request must observe the latest committed branch and context epoch, including mid-tool-loop compaction.
2. Typed evidence cursors, leases and segmented catalogs default **on in AgentSession** despite a stale SDK comment. Episodic pointers are automatically added to requests despite stale tool wording. Keep defaults and status based on executable paths.
3. Old autonomous runner types/stubs are not an active autonomy implementation. MCP client/DAG runner have incomplete production wiring; `/workflow` steers text rather than calling the DAG runner. Classify these honestly.
4. Memory, task state, episodes, artifacts, journals and native runtime observations are distinct state families. Do not flatten them into a UI chat log.
5. TS process cwd/provider/scheduler globals constrain multi-session bridges. Do not launch independent worker schedulers against a one-slot local model without a shared authority.
6. Existing SDK/RPC/Godot sidecar seams justify a temporary TS worker. The final core converges on Rust. Every bridge phase needs a retirement/compatibility criterion and a clear single writer.
7. Dyad's actual root license is Apache-2.0 outside `src/pro`, despite MIT package metadata. `src/pro` is separately FSL-licensed and excluded by VISION. Portable files can import restricted implementations/types; transitive closure and asset provenance must be audited.
8. Dyad preview contains both iframe/instrumentation and Electron WebContentsView/CDP paths. Do not promise a mechanical Tauri conversion. Separate webview display from browser automation and native launch/observation.
9. B01 aggregate result cap, B02 corrupt memory overwrite, B03 text/hash snapshot race and B04 forked artifact closure must have explicit corrected-behavior regression tests. Old defective output is not the golden oracle.
10. Rust/Tauri lightweight claims need matched measurements including every process. Synthetic microbenchmarks and `--version` startup are not full application measurements.

## Decisions to make during planning

Use TARGET_ARCHITECTURE as an evidence-backed recommendation, not immutable code design. Finalize the smallest coherent crate/module boundaries, core host lifetime, protocol schema strategy, transactional storage/import strategy, extension compatibility scope, preview capabilities and performance gates. For a deviation, record the evidence, affected parity IDs/invariants, tradeoff and changed tests.

The default recommendation is an embedded GUI-independent Rust library behind a thin Tauri host, with supervised model/project/browser resources, and a temporary supervised TS worker. A daemon is deferred unless a concrete task-lifetime requirement warrants it. Window close, renderer crash, application quit and process crash need distinct semantics. Core state independence does not by itself imply tasks run after host-process death.

Specify protocol version/schema negotiation, command identity/idempotency, snapshots and subscription watermark, event sequence/epoch, reconnect, cancellation, approvals bound to operation and revision, typed errors, bounded queues/backpressure and bounded artifact streaming. Tauri channels/events are transports, not durable state or permission checks. Generated/previewed applications receive no AG1 command authority or secrets.

Specify one authority per state family through every phase. Import journals/memory/artifacts/keys/unknown custom records into a new destination; preserve raw originals and rollback. No opportunistic dual writes. Forks must retain authorized artifact closure. Filesystem mutations, Git history, session branching and runtime observations are different transactions.

## Required planning outputs

Create `ROADMAP.md` in this workspace and a directory such as `phases/` containing an index plus one Markdown instruction file per derived phase. Phase count follows the real dependency graph; do not start with an arbitrary number and force work into it. Optional decision records belong under `planning/` and must be linked.

`ROADMAP.md` must include:

- Concrete product outcome and final ownership/process model.
- Scope/status dispositions for every A01–A85 row, including adjacent clients and legacy/partial paths. Refinements may create child IDs, not delete parent capabilities.
- Ordered phases, explicit dependency graph, authority handoffs and intermediate usable products.
- Entry/exit gates, deliverables, tests, risk mitigations, performance and licensing gates.
- A mapping from parity rows and invariants to phases and test evidence; every confirmed bug maps to a regression phase.
- Data migration, compatibility/export/rollback and reference-retirement strategy.
- Missing measurements/platform experiments with owners, methods and decision criteria. Do not invent results.
- Definition of done for planning and, separately, for final implementation.

Each phase file must be executable by a fresh model instance with no conversation history. Include:

1. Objective and exact user-visible/core behavior delivered.
2. Required reading and pinned references, including relevant parity/invariant/defect IDs.
3. Prerequisite phases and concrete files/APIs/tests that must exist; fail the entry gate if they do not.
4. Allowed workspace and original-repository protection rules.
5. Current owner → target owner and exact authority/cutover boundary.
6. Bounded scope, excluded unrelated work and evidence-based implementation instructions.
7. Interfaces/data formats/errors/cancellation/persistence changes with migration/compatibility obligations.
8. Approved frontend reuse files and notices when applicable; no Pro import assumptions.
9. Specific test fixtures, commands, expected assertions, failure injection and regression criteria.
10. Performance workload/budget/method and limitations, proportional to the change.
11. Objective completion checklist and required evidence/report artifacts.
12. Rollback/recovery steps and clearly identified unresolved blockers.
13. A concise handoff summarizing what the next phase may rely on.

Avoid instructions such as “implement the remaining core,” “ensure parity” or “test thoroughly” without concrete ownership, scenarios and completion evidence. Do not allow a phase to depend on unimplemented future services through placeholders. A fake host is useful only when the phase explicitly delivers and tests it; label it as fake.

## Plan review and stopping condition

Before finishing, validate that every required research artifact was considered, important references resolve, phases are topologically executable, all A01–A85/I01–I30/B01–B04 IDs have dispositions, no state family has two writers, the UI remains Dyad-derived but AG1-native, and the final runtime remains Rust-owned and GUI-independent. Include tests for each phase, quality gates that cannot be traded for speed, license/provenance gates and recovery/long-session acceptance.

Keep uncertainty explicit and bounded. Resolve routine implementation choices using the research. Ask for missing user requirements only when they materially prevent a coherent plan; do not ask permission for ordinary read-only verification or document creation.

**Finish by reporting the planning files and material unresolved decisions. Stop. Do not scaffold the app, start the Rust rewrite, execute phases, modify Cruzible, deploy, publish or open external communications as part of this planning turn.**
