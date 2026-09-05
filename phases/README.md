# Implementation phase instructions

These files are a plan, not evidence of completed implementation. Planning stopped after producing them. Start implementation only when instructed to execute a phase.

Read `../VISION.md`, `../MASTER-PROMPT.md`, `../ROADMAP.md`, `../planning/DECISIONS.md`, `../planning/LLAMA_CPP_PRESERVATION.md`, `../planning/COVERAGE.md`, this file and the selected phase. Every phase identifies its additional repository/test references. Resolve A/G/M/T source prefixes through `research/README.md` in the workspace root.

Workspace is `/home/alex/Projects/AG1`. Cruzible (`/home/alex/Projects/Cruzible`), `vendor/dyad`, the three llama runtime directories and original user config/weights are read-only references. Preserve pre-existing changes. Source pins: AG1 `e30f0b4b238ee5618928bcd17b4571eb046e7038`, Dyad `fb17039bfd2adcac7d74b9c1178a3a3524df35b6`. Reverify HEAD/hash deltas before implementation; do not silently refresh the baseline.

## Common execution contract

1. Check every dependency's `implementation/evidence/Pxx/completion.json`, referenced test logs and produced APIs/files. A missing dependency is an entry failure. Do not create a stub and claim the dependency exists.
2. Proposed paths and commands in these phase files **do not exist yet** unless evidence proves otherwise. The phase that names a new executable/test owns creating it. P01 creates `tools/phase_gate.py`; P02 creates the Rust workspace and protocol tests; P04 creates the React test scripts. Gate runner must invoke real tests/checks, preserve their exit status, and reject empty test selections, skipped mandatory gates, stale input hashes and missing evidence. A checklist-only validator cannot pass a phase.
3. Write implementation under `product/`, reproducible fixtures under `fixtures/`, isolated reference copies under `reference/`, harnesses under `tools/`, and phase reports under `implementation/evidence/Pxx/`. Only edit other planning files to record justified deviations/coverage updates. No blanket permission to change originals, credentials or tuned runtimes.
4. Keep L01–L08 in every handoff. Observe/import runtime settings read-only; never rebuild/upgrade/retune or fall back to stock llama.cpp. Fake process/provider tests are the default. Private launch/credential material belongs in local ignored storage, with redacted manifests in reports.
5. Before real mutations or inference, establish the phase's single writer and endpoint owner. Shadow ports consume frozen captures in isolated directories; never execute a Rust shadow tool against the same working tree as TS.
6. Every phase produces a completion report with input revisions/hashes, files/APIs delivered, authority before/after, tests and exact commands/results, coverage IDs, licensing changes, performance scope, residual gaps, rollback procedure and next-phase guarantees. `completion.json` has `phase`, `status`, `input_fingerprints`, `dependencies`, `artifacts`, `commands`, `coverage`, `authority`, `pending_gates`; status can be passed only with required evidence present. Record pending platform/live gates explicitly.
7. Run relevant format/lint/build/test checks on changed product/harness code. Do not run Cruzible's writing formatter. Do not repeat live/long tests for a cosmetic change; do run them at authority/release gates.
8. Preserve an executable reference and rollback checkpoints until P14 acceptance. No silent feature deletion, unknown-state normalization or changed tuning counted as optimization. Stop and report a material failure; do not advance to a dependent phase with a failed gate.

## Order and outputs

| Phase | Prerequisites | Coherent result |
|---|---|---|
| [P01](P01-reference-and-runtime-lock.md) | Research | Pinned reference, tuned runtime manifest, fixtures, provenance and executable gate harness |
| [P02](P02-protocol-and-core-boundary.md) | P01 | Rust core/protocol foundation and fake host with tested ordering |
| [P03](P03-platform-services.md) | P02 | Explicit workspace/files/Git/process/PTY primitives |
| [P04](P04-typescript-bridge-and-desktop.md) | P03 | Real AG1 desktop through guarded TS worker; early preview feasibility |
| [P05](P05-storage-and-import.md) | P02 | Independent Rust persistence/import/export with fault recovery |
| [P06](P06-providers-and-preserved-llama.md) | P03,P05 | Rust provider/scheduler and exact tuned-runtime launch plans |
| [P07](P07-task-and-evidence-state.md) | P03,P05 | Task/memory/artifact/source snapshot APIs |
| [P08](P08-retrieval-and-context.md) | P06,P07 | Deterministic retrieval/episodes/compiler in shadow |
| [P09](P09-tools-policy-and-resources.md) | P06,P07 | Tools, approvals, mutations, resources and extension broker |
| [P10](P10-session-runtime-and-recovery.md) | P08,P09 | Complete Rust session loop/recovery/subagents on isolated fixtures |
| [P11](P11-project-and-native-runtimes.md) | P04,P10 | Web/native/Tauri/Godot runtime adapters |
| [P12](P12-ui-and-client-parity.md) | P11 | Complete AG1 UI projections and non-GUI/compatibility clients |
| [P13](P13-authority-cutover.md) | P12 | Quiescent migration to one Rust authority with rollback |
| [P14](P14-release-and-long-horizon-gates.md) | P13 | Packaged, measured, provenance-clean release candidate |

P05 can proceed after P02 independently of the early UI branch. P06/P07 and P08/P09 can be developed in dependency order without changing the executing TS desktop. This graph does not request concurrent agents or authorize concurrent local inference.
