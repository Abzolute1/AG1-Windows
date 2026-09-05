# Coverage and disposition ledger

Every parent capability is retained here. Phase ownership denotes where implementation or explicit scope disposition is delivered, not proof it is already complete. All active high-risk rows require both a successful trace and a failure fixture before P13; P14 audits evidence for every row. Source paths, current status and reference tests remain in `research/AG1_PARITY_MATRIX.md`. No adjacent/partial status is silently promoted to Rust parity.

The machine-readable companion is `coverage.json`. P01 refines broad families (provider variants, tools, persistence versions, client actions) into child cases while preserving parent IDs. Existing test names are fixture inputs, not a claim that an identically named Rust test exists.

| ID | Capability | Delivery/disposition phase | Disposition | Reference test inputs | Verification chain |
|---|---|---|---|---|---|
| A01 | CLI/config/mode bootstrap | P12 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | `main-cli-contract.test.ts`, `args.test.ts`, `stdout-cleanliness.test.ts` | P01 capture → P12 contract/failure tests → P13 integration → P14 acceptance |
| A02 | SDK construction/resume | P10 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | `sdk-session-manager.test.ts`, `sdk-skills.test.ts` | P01 capture → P10 contract/failure tests → P13 integration → P14 acceptance |
| A03 | Session replacement | P10 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | `suite/agent-session-runtime.test.ts`, `session-cwd.test.ts` | P01 capture → P10 contract/failure tests → P13 integration → P14 acceptance |
| A04 | Turn/tool loop | P10 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | `packages/agent/test/` loop tests; new normalized transcript oracle | P01 capture → P10 contract/failure tests → P13 integration → P14 acceptance |
| A05 | Steering/follow-ups | P10 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | `suite/agent-session-queue.test.ts`, `interactive-queued-messages.test.ts` | P01 capture → P10 contract/failure tests → P13 integration → P14 acceptance |
| A06 | Cancellation/settlement | P10 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | `agent-session-dispose-recovery.test.ts`, `agent-session-continue-replay.test.ts` | P01 capture → P10 contract/failure tests → P13 integration → P14 acceptance |
| A07 | Ordered concurrent tool batches | P10 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | `packages/agent/test/`, `agent-session-concurrent.test.ts` | P01 capture → P10 contract/failure tests → P13 integration → P14 acceptance |
| A08 | Event drain/persistence barrier | P10 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | `agent-session-context-boundary.test.ts`, `listener-hardening.test.ts` | P01 capture → P10 contract/failure tests → P13 integration → P14 acceptance |
| A09 | Runtime retry/replay | P10 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | retry/arbiter/replay-scheduler tests | P01 capture → P10 contract/failure tests → P13 integration → P14 acceptance |
| A10 | Malformed/XML tool recovery | P10 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | `xml-tool-parser.test.ts`, `agent-session-xml-coordinator.test.ts` | P01 capture → P10 contract/failure tests → P13 integration → P14 acceptance |
| A11 | Repetition/failure/stop guards | P10 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | repetition/tool-failure/turn-health tests | P01 capture → P10 contract/failure tests → P13 integration → P14 acceptance |
| A12 | Controlled runtime policy | P10 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | `task-policy.test.ts`, `tool-orchestrator.test.ts`, controller tests | P01 capture → P10 contract/failure tests → P13 integration → P14 acceptance |
| A13 | Coding completion gate | P10 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | `coding-completion-gate.test.ts`, `print-mode.test.ts` | P01 capture → P10 contract/failure tests → P13 integration → P14 acceptance |
| A14 | Per-request authoritative context | P10 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | `agent-session-context-boundary.test.ts`, `agent-session-auto-compaction-queue.test.ts` | P01 capture → P10 contract/failure tests → P13 integration → P14 acceptance |
| A15 | Deterministic working-set compiler | P08 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | `context-compiler.test.ts`, `prefix-stability.test.ts` | P01 capture → P08 contract/failure tests → P13 integration → P14 acceptance |
| A16 | Ingestion externalization | P09 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | `tool-result-artifacts.test.ts`, `agent-session-artifacts.test.ts` | P01 capture → P09 contract/failure tests → P13 integration → P14 acceptance |
| A17 | Manual/automatic summarization | P10 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | compaction coordinator/queue/serialization tests | P01 capture → P10 contract/failure tests → P13 integration → P14 acceptance |
| A18 | Hygiene/emergency compaction | P08 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | `micro-compact.test.ts`, compaction tests | P01 capture → P08 contract/failure tests → P13 integration → P14 acceptance |
| A19 | System prompt/resources/addenda | P08 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | `system-prompt-grounding.test.ts`, `turn-addenda.test.ts`, prompt tests | P01 capture → P08 contract/failure tests → P13 integration → P14 acceptance |
| A20 | Token accounting/overflow | P08 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | `session-metrics.test.ts`, context-boundary tests | P01 capture → P08 contract/failure tests → P13 integration → P14 acceptance |
| A21 | Prefix/cache stability | P08 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | `prefix-shape.test.ts`, `prefix-stability.test.ts` | P01 capture → P08 contract/failure tests → P13 integration → P14 acceptance |
| A22 | Task objective/phase/lists | P07 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | `task-state.test.ts`, `agent-session-task-state.test.ts` | P01 capture → P07 contract/failure tests → P13 integration → P14 acceptance |
| A23 | Epistemic claims/provenance | P07 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | `task-state-executive.test.ts`, `agent-session-executive-state.test.ts` | P01 capture → P07 contract/failure tests → P13 integration → P14 acceptance |
| A24 | Checkpoint verification | P07 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | `task-state-executive.test.ts`, `runtime-observation.test.ts` | P01 capture → P07 contract/failure tests → P13 integration → P14 acceptance |
| A25 | Todos | P07 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | `todos.test.ts`, `todo-integration.test.ts`, `agent-session-todos.test.ts` | P01 capture → P07 contract/failure tests → P13 integration → P14 acceptance |
| A26 | Plan submission/questions/mode | P10 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | `submit-plan-tool.test.ts`, interactive-plan tests | P01 capture → P10 contract/failure tests → P13 integration → P14 acceptance |
| A27 | Runtime/inspection evidence | P07 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | runtime-observation/inspection-evidence tests | P01 capture → P07 contract/failure tests → P13 integration → P14 acceptance |
| A28 | Explicit episodes | P08 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | `episode-memory.test.ts`, agent-session-episode tests | P01 capture → P08 contract/failure tests → P13 integration → P14 acceptance |
| A29 | Episode drafting | P08 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | `episode-candidate.test.ts`, `isolated-model-call.test.ts` | P01 capture → P08 contract/failure tests → P13 integration → P14 acceptance |
| A30 | Episode retrieval/pointers | P08 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | `episode-retrieval.test.ts`, `agent-session-episode-context.test.ts` | P01 capture → P08 contract/failure tests → P13 integration → P14 acceptance |
| A31 | CV feature switches | P08 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | `cv-architecture.test.ts` | P01 capture → P08 contract/failure tests → P13 integration → P14 acceptance |
| A32 | Embeddings/learned rerank/delta protocol/general allocator GC | P01 | Absent/planned: exclude new algorithms; keep status explicit. | new quality/perf experiment before adoption | P01 status evidence → P01 disposition/API tests → P14 scope audit |
| A33 | Lexical/exact-symbol selection | P08 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | `source-context-tool.test.ts`, replay trajectory | P01 capture → P08 contract/failure tests → P13 integration → P14 acceptance |
| A34 | Vague follow-up/changed-file recovery | P08 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | source-retrieval-signals, context-replay tests | P01 capture → P08 contract/failure tests → P13 integration → P14 acceptance |
| A35 | Source capsules/relations | P08 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | `context-capsule.test.ts`, `source-context-tool.test.ts` | P01 capture → P08 contract/failure tests → P13 integration → P14 acceptance |
| A36 | Source revision validation | P07 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | `source-revision.test.ts`; new B03 race fixture | P01 capture → P07 contract/failure tests → P13 integration → P14 acceptance |
| A37 | Repo map/index/manifests | P03 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | repo-map/workspace-index tests | P01 capture → P03 contract/failure tests → P13 integration → P14 acceptance |
| A38 | Memory store/migration/archive | P05 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | `memory-store.test.ts`; new B02 fault tests | P01 capture → P05 contract/failure tests → P13 integration → P14 acceptance |
| A39 | Memory injection/freshness | P08 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | `memory-context.test.ts`, `turn-addenda.test.ts` | P01 capture → P08 contract/failure tests → P13 integration → P14 acceptance |
| A40 | Immutable text/binary artifacts | P05 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | `artifact-store.test.ts`, `artifact-tool.test.ts` | P01 capture → P05 contract/failure tests → P13 integration → P14 acceptance |
| A41 | Artifact paging/grep/search | P05 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | artifact tests incl UTF-8/CRLF | P01 capture → P05 contract/failure tests → P13 integration → P14 acceptance |
| A42 | Typed evidence cursors | P05 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | cursor tests in artifact-store suite | P01 capture → P05 contract/failure tests → P13 integration → P14 acceptance |
| A43 | Evidence leases | P05 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | artifact-store + context-compiler lease tests | P01 capture → P05 contract/failure tests → P13 integration → P14 acceptance |
| A44 | Segmented catalog/reconstruction | P05 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | catalog fault-injection tests in artifact-store | P01 capture → P05 contract/failure tests → P13 integration → P14 acceptance |
| A45 | Binary retention/visual rehydration | P11 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | artifact tests + Godot runtime tests | P01 capture → P11 contract/failure tests → P13 integration → P14 acceptance |
| A46 | Session journal/tree | P05 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | `session-manager/` suite | P01 capture → P05 contract/failure tests → P13 integration → P14 acceptance |
| A47 | Session corruption/migrations/concurrent writes | P05 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | migration/lock/save-entry tests; new power-loss tests | P01 capture → P05 contract/failure tests → P13 integration → P14 acceptance |
| A48 | Branch/fork/summarize/export | P10 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | branching/tree-navigation/branch-summarization tests | P01 capture → P10 contract/failure tests → P13 integration → P14 acceptance |
| A49 | Auth/settings/config secrets | P06 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | auth/settings/secret resolution tests | P01 capture → P06 contract/failure tests → P13 integration → P14 acceptance |
| A50 | Multi-provider abstraction | P06 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | `packages/ai/test/`, dynamic-provider tests | P01 capture → P06 contract/failure tests → P13 integration → P14 acceptance |
| A51 | OAuth/credential refresh | P06 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | auth-storage and provider-specific tests | P01 capture → P06 contract/failure tests → P13 integration → P14 acceptance |
| A52 | Local endpoint/model discovery | P06 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | model-registry/resolver/local runtime tests | P01 capture → P06 contract/failure tests → P13 integration → P14 acceptance |
| A53 | llama.cpp lifecycle | P06 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | `llamacpp-runtime.test.ts`; new PID/port ownership tests | P01 capture → P06 contract/failure tests → P13 integration → P14 acceptance |
| A54 | Model profiles/VRAM/KV tuning | P06 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | `llamacpp-vram-tuning.test.ts`, runtime tests | P01 capture → P06 contract/failure tests → P13 integration → P14 acceptance |
| A55 | Capability/thinking/vision and switching | P06 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | capability/thinking/vision/model-switch tests | P01 capture → P06 contract/failure tests → P13 integration → P14 acceptance |
| A56 | Streaming/watchdogs/fallbacks | P06 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | loop/listener/nvidia/AI stream tests | P01 capture → P06 contract/failure tests → P13 integration → P14 acceptance |
| A57 | Inference scheduling | P06 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | `inference-scheduler.test.ts` | P01 capture → P06 contract/failure tests → P13 integration → P14 acceptance |
| A58 | Isolated model calls | P06 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | `isolated-model-call.test.ts`, episode candidate tests | P01 capture → P06 contract/failure tests → P13 integration → P14 acceptance |
| A59 | Tool definitions/extension hooks | P09 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | dynamic-tools/extensions/tool tests | P01 capture → P09 contract/failure tests → P13 integration → P14 acceptance |
| A60 | Approval/safety/planning policy | P09 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | `safety-policy.test.ts`, headless/trust tests | P01 capture → P09 contract/failure tests → P13 integration → P14 acceptance |
| A61 | Read/find/grep/list | P09 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | `tools.test.ts`, path-utils/web context tests | P01 capture → P09 contract/failure tests → P13 integration → P14 acceptance |
| A62 | Edit/write/per-file queue | P09 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | file-mutation-queue/tools/edit legacy tests | P01 capture → P09 contract/failure tests → P13 integration → P14 acceptance |
| A63 | Append/diff/edit preview/apply | P09 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | tools/edit-preview oracles; new CAS/atomic tests | P01 capture → P09 contract/failure tests → P13 integration → P14 acceptance |
| A64 | Shell execution/output | P09 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | bash output/persistence/close-hang tests | P01 capture → P09 contract/failure tests → P13 integration → P14 acceptance |
| A65 | Background jobs | P09 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | job-registry/job-tool/job-wait tests | P01 capture → P09 contract/failure tests → P13 integration → P14 acceptance |
| A66 | Sandbox/workspace isolation | P09 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | sandbox/bash-sandbox/auton-workspace tests | P01 capture → P09 contract/failure tests → P13 integration → P14 acceptance |
| A67 | Compiler check/diagnostics | P11 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | check-detect/parse/tool tests | P01 capture → P11 contract/failure tests → P13 integration → P14 acceptance |
| A68 | Web search/fetch | P09 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | webfetch-ssrf/credentials/web-tools-context | P01 capture → P09 contract/failure tests → P13 integration → P14 acceptance |
| A69 | Browser/visual verification | P11 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | visual-tools/engine/url/verify-ui tests | P01 capture → P11 contract/failure tests → P13 integration → P14 acceptance |
| A70 | Subagent profiles/batches/checkpoints | P10 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | `subagent.test.ts`, `explore-tool.test.ts`, delegation tests | P01 capture → P10 contract/failure tests → P13 integration → P14 acceptance |
| A71 | Skills/templates/packages/extensions | P09 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | resource/skills/package/extensions suites | P01 capture → P09 contract/failure tests → P13 integration → P14 acceptance |
| A72 | Skill creation/update helpers | P12 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | existing skill tests; new creator behavior fixture | P01 capture → P12 contract/failure tests → P13 integration → P14 acceptance |
| A73 | RPC/headless protocol | P12 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | rpc-jsonl/validation/rpc/stdout tests | P01 capture → P12 contract/failure tests → P13 integration → P14 acceptance |
| A74 | TUI/interactive commands/settings | P12 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | interactive action/plan/model/settings tests | P01 capture → P12 contract/failure tests → P13 integration → P14 acceptance |
| A75 | Usage/cost/debug/audit/export | P12 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | metrics/debug/audit/run-diagnostics tests | P01 capture → P12 contract/failure tests → P13 integration → P14 acceptance |
| A76 | Deterministic replay/quality evaluation | P14 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | context-replay-eval/trajectory tests | P01 runner/fixtures → P08/P10 replay → P14 duration/live evidence |
| A77 | Live long-horizon/bench harnesses | P14 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | original suite fixtures + new Rust runner | P01 runner/fixtures → P08/P10 replay → P14 duration/live evidence |
| A78 | Godot native embedding | P11 | Preserve active/selected behavior in Rust; corrected defects use the documented invariant. | `packages/godot-agent-runtime/test/` and native fixture contracts | P01 capture → P11 contract/failure tests → P13 integration → P14 acceptance |
| A79 | Workflow engine | P12 | Preserve textual /workflow behavior; retain unintegrated DAG API in reference, no new DAG scheduler. | new workflow invocation/unknown-dependency fixture | P01 status evidence → P12 disposition/API tests → P14 scope audit |
| A80 | MCP client | P12 | Preserve partial stdio client in reference; document optional adapter gap, no invented HTTP integration. | new stdio cancellation/schema/exit tests | P01 status evidence → P12 disposition/API tests → P14 scope audit |
| A81 | Removed autonomous runner | P01 | Removed runner stays removed; active workspace helpers are delivered under A66. | original auton workspace tests | P01 status evidence → P01 disposition/API tests → P14 scope audit |
| A82 | Web UI library | P12 | Retain separate Lit reference client; publish API status, no React/Lit UI port required. | package tests/browser smoke | P01 status evidence → P12 disposition/API tests → P14 scope audit |
| A83 | Slack mom integration | P12 | Retain separate Slack reference bot; publish API status, no desktop Slack service required. | separate package tests + recorded adapters | P01 status evidence → P12 disposition/API tests → P14 scope audit |
| A84 | Pods/GPU administration | P12 | Retain separate pods reference CLI; publish API status, no desktop cloud administration required. | separate package fixtures; new SSH fake | P01 status evidence → P12 disposition/API tests → P14 scope audit |
| A85 | Legacy SDK globals/hook export | P12 | Advertised ./hooks targets are absent at pin; document broken export. Preserve supported active extension APIs and explicit shim/errors; no cwd globals in Rust. | import/export contract tests | P01 status evidence → P12 disposition/API tests → P14 scope audit |

## Invariants

Read the exact properties in `research/INVARIANTS.md`. Each listed phase contributes named cases; P14 checks their combined evidence.

| ID | Delivery/test phases |
|---|---|
| I01 | P02, P04, P10, P12, P13 |
| I02 | P08, P10 |
| I03 | P05, P10 |
| I04 | P09, P10 |
| I05 | P05, P08, P10 |
| I06 | P03, P07, P08 |
| I07 | P07, P10, P11 |
| I08 | P07, P10 |
| I09 | P08 |
| I10 | P05, P07, P13 |
| I11 | P02, P05, P07 |
| I12 | P05, P07, P08 |
| I13 | P05, P10, P13 |
| I14 | P05, P13 |
| I15 | P05, P09, P10, P13 |
| I16 | P06, P10, P13 |
| I17 | P09, P10 |
| I18 | P03, P06, P10 |
| I19 | P03, P09, P10 |
| I20 | P02, P09, P12 |
| I21 | P03, P06, P11, P13 |
| I22 | P02, P03, P10, P12 |
| I23 | P04, P12 |
| I24 | P03, P09, P12 |
| I25 | P06, P10 |
| I26 | P04, P11, P12 |
| I27 | P05, P09, P11, P12, P13 |
| I28 | P01, P08, P12 |
| I29 | P01, P04, P06, P14 |
| I30 | P01, P04, P12, P14 |

## Preserved llama.cpp runtime

L01–L08 are defined in `LLAMA_CPP_PRESERVATION.md` and are additional binding requirements.

| ID | Delivery/test phases |
|---|---|
| L01 | P01, P04, P06, P13, P14 |
| L02 | P01, P04, P06 |
| L03 | P01, P06, P10, P14 |
| L04 | P04, P06, P12, P13 |
| L05 | P04, P06, P10, P13 |
| L06 | P03, P04, P06, P13 |
| L07 | P01, P04, P05, P13, P14 |
| L08 | P01, P06, P13, P14 |

## Confirmed defect corrections

The immutable reference retains the old behavior as evidence. P04 uses separate reviewed bridge patches; Rust correction lives in the owning phase. Never modify Cruzible or weaken the target assertion to match the bug.

| ID | Bridge safeguard | Rust owner | Integration | Correct oracle |
|---|---|---|---|---|
| B01 | P04 | P09 | P10 | Aggregate result projection bounded; complete evidence recoverable. |
| B02 | P04 | P05 | P13 | Corrupt memory retained and diagnosed; no empty overwrite. |
| B03 | P04 | P07 | P08 | Displayed bytes and hash from one validated snapshot. |
| B04 | P04 | P05 | P10 P13 | Fork/import keeps authorized artifact/cursor closure after restart/retention. |

D01 process identity and D03 mutation durability are additional hardening gates in P03/P04/P06 and P09. D02/D04/D05/D06/D08/D12–D14 have fault/bounds/recovery coverage in P02/P03/P05/P07/P10/P14. D07 optimization is conditional on profiling. D09/D10/D11 require policy/status/compatibility tests in P01/P08/P10/P12. D15/D16 close through provenance and preview platform gates P04/P11/P14.
