# Astra model-facing interface review — 2026-09-11

> Historical evidence, archived 2026-09-13. Status, commands, permissions and paths below describe their original checkpoint. For current work start with [implementation status](../../../README.md). Original bytes and old paths are preserved in the [relocation manifest](../manifest.json) and recovery archive.

Follow-up: the [surgical pass](ASTRA-SURGICAL-INTERFACE-20260911.md) is now implemented, installed and live verified with Luna and Qwen35B. The assessment below records the earlier state.

Assessment only: source inspected; no runtime, prompt, schema, CV budget or installed-package changes made. Astra reviewed the exposed interface; the findings below distinguish that report from implementation evidence.

Most improvements can be incremental. No CV engine rewrite is required. Schema changes must preserve old journal replay, imported records, scoped authority, exact artifact provenance and existing local-model behavior.

| Area | Implementation finding | Scope of sensible change |
| --- | --- | --- |
| Browser prerequisite | SharedProject.names advertises browser tools when the browser is installed, even before a preview exists. Runtime discover/start is a client command, not an advertised model tool. Execution correctly rechecks the owned preview grant and selected model vision. | Small to clarify host-supplied readiness and remove duplicate text; moderate to expose a narrow lifecycle operation or make tool availability refresh dynamically. Reuse existing lifecycle and grants. |
| Editing overlap | edit uses original-content disjoint edits; editApply uses sequential matching; editPreview is read-only and does not issue an apply token. Fuzzy is opt-in Unicode/typography/trailing-space normalization, not semantic matching. | Small to advertise one preferred editor and explain preview as optional; moderate to add a common facade. A revision-bound preview/apply token is a separate moderate feature, not necessary for the common path. Preserve historical tool names/semantics for replay. |
| Evidence retrieval | Existing artifact service already searches, returns bounded surrounding context and supplies readCall cursors; native prompt repeats the operational rules. | Moderate additive retrieval convenience operation over existing storage: resolve a reference/query, select context, enforce one total budget, return source references and continuation only when needed. Do not replace storage, exact reads, leases or CV compiler. |
| Verification leases | A genuine successful built-in check releases all release_on_verification leases in the session. No claim/task/check-scope predicate exists. Hooks cannot forge the trigger and arbitrary passing bash commands do not trigger it. | Focused but moderate correctness change: explicit obligation/scope binding plus receipt match, conservative handling of old unscoped leases, and tests covering unrelated passing checks. Storage schema, import/fork/copy paths and replay need attention. |
| Work records | runtimeObservation already appends related task-state observations atomically, retains provenance, and reopens completed tasks for related non-passes. Relatedness is a term-match heuristic, not an explicit obligation identity. | Small to document the evidence/task/memory hierarchy and make optional records optional; moderate to extend explicit event projections. Replacing all state stores with one universal event system would be a large refactor and is unnecessary. |
| Todo | Three states, one active step, prospective new plans; archive/new-plan already exist. Existing pending items can become completed without a separate activation call, so Astra's activation-before-completion assertion is not universally accurate. | Small to define it as coarse sequential phases; moderate for new statuses across validators, UI, projections and import/replay. Do not turn it into a scheduler merely to mirror background jobs. |
| Delegation/board | explore uses the native read-only delegation machinery. Its board is persisted by workspace, retains 12 findings and injects the last 8 question/summary pairs. Injection omits stored timestamp/evidence details; no revision freshness check occurs in that prior-summary formatter. Workers receive a prior board snapshot, not live peer writes during the same batch. | Small to explain actual lifecycle; moderate to carry source/revision/age and mark prior summaries historical. A unified delegation facade is optional; rebuilding child ownership/scheduling is unnecessary. |
| GitHub wording and repeated instructions | Tool description literally says “Copy schema.example as the complete arguments”. screenshot_url duplicates its restriction because native_tools concatenates the same guidance twice. Descriptions, per-tool native prompt and verification addendum overlap. | Small wording/deduplication changes. Use examples' structure and replace placeholders. Preserve enforcement in code and essential boundaries; measure the rendered prompt, then verify local Qwen and subscription behavior. |

Highest priority: scope lease release; fix browser prerequisite discoverability; fix stale exploration-board presentation. Then simplify the common edit/evidence paths and remove redundant instructions. Keep current CV ratios, thresholds, evidence retention and tuned local-model runtime settings unchanged while testing the interface changes.

Prompt edits are small in code but can change local-model behavior. Acceptance should cover the actual rendered tool surface, missing/ready preview transitions, scoped lease isolation, exact retrieval/provenance under budget pressure, old edit replay and one bounded local Qwen plus cloud-model scenario. This review did not run those future-change tests.

Source anchors:

- [Browser discovery before readiness](../../../../product/crates/ag1-platform/src/projects/target.rs:472)
- [Native descriptions and duplicate browser guidance](../../../../product/crates/ag1-core/src/tools/native_prompt.rs:10)
- [Host project lifecycle commands](../../../../product/crates/ag1-platform/src/client/projects.rs:97)
- [Edit matching implementations](../../../../product/crates/ag1-core/src/tools/file.rs:129)
- [Fuzzy normalization](../../../../product/crates/ag1-core/src/tools/edit.rs:44)
- [Artifact search/context service](../../../../product/crates/ag1-platform/src/artifact_tools.rs:231)
- [Successful native check trigger](../../../../product/crates/ag1-core/src/tools/executor.rs:291)
- [Session-wide lease release](../../../../product/crates/ag1-storage/src/ports.rs:59)
- [Existing observation/task projection](../../../../product/crates/ag1-core/src/tools/observation.rs:56)
- [Todo lifecycle](../../../../product/crates/ag1-core/src/todo.rs:52)
- [Persistent exploration board](../../../../product/crates/ag1-platform/src/session/subagent_tool/explore.rs:21)
- [Board prior-summary projection](../../../../product/crates/ag1-core/src/runtime/subagent/explore.rs:46)
- [GitHub tool wording](../../../../product/crates/ag1-platform/src/github/tool.rs:11)
