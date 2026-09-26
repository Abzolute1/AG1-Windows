# P12 fresh-thread handoff

> Historical evidence, archived 2026-09-13. Status, commands, permissions and paths below describe their original checkpoint. For current work start with [implementation status](../../../README.md). Original bytes and old paths are preserved in the [relocation manifest](../manifest.json) and recovery archive.

Start P12 only when explicitly requested. P11's status and exact acceptance evidence are [completion.json](../../P11/completion.json), [REPORT.md](../../P11/REPORT.md), [PROJECTS.md](../../../../product/crates/ag1-platform/PROJECTS.md), [performance.json](../../P11/performance.json) and its immutable delivered snapshot. Read the P12 plan and common readings in phases/README.md. The user requested a P11 commit and stop; no P12 implementation is authorized by this handoff alone.

From `/home/alex/Projects/AG1`:

```sh
git status --short
git log -3 --oneline
python3 -c 'import sys; sys.path.insert(0,"tools"); from phase_gate import verify_dependencies; verify_dependencies("P12"); print("P11 artifacts verified")'
python3 -c 'import sys; sys.path.insert(0,"tools"); from common import verify_original; from runtime_gate import preservation; verify_original(); preservation(); print("Original pins and tuned runtime verified")'
```

A dependency hash check does not certify current source/runtime fingerprints. Do not silently replace a changed baseline. Preserve all earlier evidence; P12 writes its own acceptance evidence. The executable reference remains `reference/ag1-executable`, with pinned Node `.local/toolchains/node-v24.14.0-linux-x64/bin/node`.

P11 APIs: `ag1_core::projects` views; `ag1_platform::projects::Runtime` owns an explicit workspace plus separate private catalog/log destination and single writer; create/load/discover/command/start/poll/finish/stop/log/history, private display and reviewed Git recovery. `CheckPort`, `VisualPort` and `ProjectPort` implement P09 target capability preparation/revalidation/execution and integrate through `NativeAdapters.project`. `NativeSession` keeps the existing policy and durable execution fences. Godot's v1 schema/nonce/allowlist/transactions/watchdog/capture/stage adapter keeps native host authority. `preview::Browser` is optional independent automation; `preview::retain` uses P05 branch/fork binary custody. `ag1_desktop::project_preview::ProjectPreview` is a host-only incognito WebKit boundary with focus/close and constrained navigation.

The ordinary desktop still uses its TypeScript worker. P12 owns full React/client projections/actions, target lifecycle controls and log/problems presentation, CLI/JSONL/SDK facades, HTML export presentation and extension/provider/UI routing. P13 owns authority transfer. P10's extension DenyBridge/dynamic-rebind boundaries remain in force. Do not create a hidden TypeScript parent AgentSession in native clients.

Evidence distinctions: real Linux Vite/browser interaction and frontend failures; actual Rust/Cargo runtime/compiler failures; actual Tauri/WebKit native render, interaction and crash; actual isolated native WebKit dev-server preview; fake custom-Godot host protocol/transaction/watchdog/staging and private PNG evidence. Native embedding is unavailable. Missing compilers/browsers/platforms return actionable status. TypeScript checks accept contained file arguments; other compilers check the complete target. Judge-only vision is unavailable in this binding; caller-vision assessment works. Any future judge/model call must use the shared P06 scheduler, never the browser helper. Native window focus requires an owned window handle; the desktop WebKit preview has one, arbitrary native processes do not.

`python3 tools/phase_gate.py P11` is a real gate. The project contract needs the `project-native-fixture` example built with `--features visual-test`, pinned optional Chromium/Playwright and local Xvfb at paths/hashes in `fixtures/projects/tooling-lock.json`. Xvfb was downloaded/extracted solely under `.local/p11`; it is not a system installation or product dependency. Its package URL/hash is recorded. Other-OS, live-model, final renderer latency, packaging and duration gates remain explicit. Restore missing fixture prerequisites from the recorded pins; never substitute a model engine.

L01–L08 remain binding:

- L01: preserve `/home/alex/Projects/llama-cpp-turboquant/build/bin/llama-server`, `/home/alex/Projects/llama-cpp-qwen4exp-mtp-b10715/llama-server`, `/home/alex/Projects/llama-cpp-qwen4exp/build/bin/llama-server` and their libraries. No rebuild, upgrade or stock fallback.
- L02: retain raw/absent/false/zero profile fields, per-model binary selection, environment precedence, context tiers and ordered arguments.
- L03: preserve GGUF shards/projectors, aliases, context/output limits, templates, reasoning/tool formats, sampling/cache/stop and speculation. Vision requires the actual selected model/projector capability.
- L04: no implicit tuning/profile writes. `/home/alex/.ag1/agent/llamacpp-model-profiles.json` remains SHA-256 `aa02688c4d39ef73234f0ec381eed22dbe02463881626423049b75b5a12ad0d0`.
- L05: one inference owner. Fixtures use owned fake loopback providers; do not compete with the TS desktop on a live endpoint.
- L06: readiness is not ownership. Stop only currently owned groups; no PID/port adoption or external server eviction.
- L07: originals, user configuration, weights and runtime trees stay read-only. Use private new destinations, retain existing user changes/recovery evidence and keep secrets out of reports.
- L08: compare matched backend/config/model/hardware/workloads. Fake fixture timings do not prove live quality or speed gains.

Cruzible remains read-only at `e30f0b4b238ee5618928bcd17b4571eb046e7038`, Dyad at `fb17039bfd2adcac7d74b9c1178a3a3524df35b6`. The preservation contract and manifests remain authoritative.
