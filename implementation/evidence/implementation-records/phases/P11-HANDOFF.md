# P11 fresh-thread handoff

> Historical evidence, archived 2026-09-13. Status, commands, permissions and paths below describe their original checkpoint. For current work start with [implementation status](../../../README.md). Original bytes and old paths are preserved in the [relocation manifest](../manifest.json) and recovery archive.

Start here when explicitly instructed to execute phase 11. This document prepares that work; P11 has not started. Read the [P11 plan](../../../../phases/P11-project-and-native-runtimes.md) and all common readings in [phase instructions](../../../../phases/README.md) before implementation.

## Verified starting point

P10 was completed and committed as `83f366b` (`P10: deliver native session runtime, recovery and subagents`). The working tree was clean after that commit. On 2026-09-07, P11's dependency check passed for P04 and P10, including every artifact hash in their completion manifests. Recheck current state at the start of the new thread; this observation does not certify later changes.

- P10 authority: [completion.json](../../P10/completion.json), [report](../../P10/REPORT.md), [current session API guide](../../../../product/crates/ag1-platform/SESSION.md) and [compatibility scope](../../../../fixtures/runtime/scope.json).
- P04 authority: [completion.json](../../P04/completion.json), [report](../../P04/REPORT.md), [limitations](../../P04/limitations.json) and [desktop development instructions](../../../../product/apps/desktop/README.md).
- P09 authority: [completion.json](../../P09/completion.json) and [report](../../P09/REPORT.md). The opening status paragraph in the older `ag1-core/TOOLS.md` still describes work in progress; use its API details with the passed completion evidence, not as a pending-phase ledger.

The old P10 continuation notes are archived in [history](../../P10/HISTORY.md). Do not redo their completed checklist or mistake a standalone differential report for phase acceptance. Preserve prior evidence and delivered snapshots; write new acceptance evidence under `implementation/evidence/P11/`.

From `/home/alex/Projects/AG1`, begin with:

```sh
git status --short
git log -3 --oneline
python3 -c 'import sys; sys.path.insert(0,"tools"); from phase_gate import verify_dependencies; verify_dependencies("P11"); print("P04/P10 dependency artifacts verified")'
```

This checks dependency status and artifact hashes, not all current source/runtime fingerprints. Inspect the delivered APIs and referenced logs, and reverify original pins and runtime material using the existing preservation helpers before implementation. Do not refresh a changed baseline silently. The executable reference is `reference/ag1-executable`; the pinned Node executable is `.local/toolchains/node-v24.14.0-linux-x64/bin/node`.

## Existing integration points

P11 supplies concrete project target implementations through the existing ownership and policy boundaries:

| Existing component | Use in P11 |
| --- | --- |
| P03 explicit workspace, files, Git, process/PTY and owned resource groups | Own fixture operations, process trees and teardown; never infer ownership from a listening port or historical PID. |
| `ag1_core::tools::targets` and `ag1_platform::target_tools::{Port, Service}` | Implement concrete target preparation, capability binding, revalidation and execution. Preparation is read-only; execution retains current approval fences and durable intent/result custody. |
| `ag1_platform::session::NativeToolRunner` and `NativeAdapters` | Compose real check/verify/visual target ports with the existing native tools. P09/P10 fake target contracts do not prove a browser, Tauri or Godot adapter exists. |
| `NativeSession::{run, continue_with, compact, navigate_tree}` | Exercise adapters through the native session when integration requires it; do not construct a hidden TypeScript AgentSession as the native executor. |
| `RuntimeRegistry`, `RuntimeCommands`, `SessionCommands` | Retain workspace-bound selection, admission/drain, command receipts, bounded progress and replay. Keep wire event sequence separate from journal revision. |
| P05 storage/artifact access and P07 verification evidence | Retain complete logs/results before projection, authorize artifact access including forks, and bind verification to actual commands, exits and current state. Compile success alone is not runtime or visual proof. |
| Shared P06 inference scheduler | Any admitted vision/model call must use the same endpoint owner as root, summary and child requests. Fixture providers are the default. |

The normal desktop still uses the TypeScript worker. P11 builds native targets and the preview boundary in isolated fixture workspaces; P13 owns desktop authority transfer. P04 proved an isolated static preview on this Linux host, with X11 and its documented DMABUF workaround. Interactive dev-server preview remains P11 work; do not assume embedded native windows, browser automation or Wayland support was proved.

P12 owns full UI/client parity, CLI/JSONL/SDK facades, HTML export presentation and final extension action/provider/UI routing. P10's optional extension binding admits static tools and callbacks, but uses `DenyBridge` for callback mutations/provider/process/UI actions and fences dynamic registration until rebind. Preserve that explicit boundary while implementing P11 targets.

## Work still required for P11

Execute the full [P11 plan](../../../../phases/P11-project-and-native-runtimes.md): web dev-server, generic native, Rust/Tauri and existing Godot host adapters; create/load/install/build/run/preview/log/stop/history; real capability reporting and Git recovery guards. Keep Godot's version/hash/nonce handshake, allowlist, transactions, staging and watchdog compatible with the pinned consumers. Record provenance for any copied code or presentation assets.

At this checkpoint `tools/phase_gate.py` has no P11 execution route. P11 must create its real `project_runtime_contract`, Godot adapter cases and gate implementation. Do not treat the planned command as an existing test or a checklist validator as acceptance. The gate must execute required tests, reject empty/skipped selections and stale/missing evidence, and write the common completion schema with exact commands and artifact/input hashes.

Cover the plan's failure cases and local workflows, including stale readiness, foreign listeners, repeated start/stop, workspace rename/delete, child leaks, unavailable browser/toolchains, compiler and runtime failures, dirty/conflicted Git operations and generated-app privilege boundaries. Record actual native/runtime evidence separately from compilation and fake-host protocol evidence. Report run-ready/log-display/stop timings separately from downloads/compiler time, with browser/target memory both separate and included in totals.

Run relevant format/lint/build/test checks and affected existing contracts. P10's full gate passed; rerun prior suites when changed shared code or unresolved evidence warrants it. Its finite fake-provider measurements do not close live per-model, other-OS, packaging or 8h/24h P14 gates. Unsupported platforms require actionable capability results; do not defer required development-OS P11 workflows under a generic P14 label.

## Tuned llama.cpp preservation: L01–L08

The [preservation contract](../../../../planning/LLAMA_CPP_PRESERVATION.md) and recorded runtime manifests control this work. Cruzible is read-only at `e30f0b4b238ee5618928bcd17b4571eb046e7038`; `vendor/dyad` is read-only at `fb17039bfd2adcac7d74b9c1178a3a3524df35b6`.

- **L01:** Preserve all three executable builds and adjacent libraries: `/home/alex/Projects/llama-cpp-turboquant/build/bin/llama-server`, `/home/alex/Projects/llama-cpp-qwen4exp-mtp-b10715/llama-server`, `/home/alex/Projects/llama-cpp-qwen4exp/build/bin/llama-server`. No rebuild, upgrade or stock fallback.
- **L02:** Preserve raw profile fields, absent/false/zero distinctions, model-specific binary selection, environment precedence, context tiers and ordered arguments.
- **L03:** Preserve GGUF shards, projectors, aliases, context/output limits, templates, reasoning/tool formats, sampling/cache/stop and speculative decoding behavior. Gate vision by actual model/projector capability.
- **L04:** No implicit tuning or profile writes. `/home/alex/.ag1/agent/llamacpp-model-profiles.json` retains SHA-256 `aa02688c4d39ef73234f0ec381eed22dbe02463881626423049b75b5a12ad0d0`.
- **L05:** Exactly one owner admits local inference. Use owned fake loopback providers for fixtures; do not compete with the TS desktop against a live endpoint.
- **L06:** Readiness is not process ownership. Stop only owned fixture groups; never adopt or kill an external server by port/PID alone.
- **L07:** Keep originals, user configuration, weights and runtime directories read-only. Use private disposable destinations, preserve pre-existing changes and rollback evidence, and keep secrets out of reports.
- **L08:** Preserve matched backend/config/model/hardware/workload baselines. Do not claim live quality or speed gains from fake fixtures or a changed backend.

## Finish and next-thread handoff

When authorized to execute P11, finish its acceptance report and completion manifest, update the implementation index, and write `implementation/P12-HANDOFF.md` with verified entry commands, delivered APIs, remaining client/UI work, known limitations, L01–L08 and the phase boundary. Commit P11 and stop before P12 when requested. A new thread must be able to continue from repository evidence without conversation history.
