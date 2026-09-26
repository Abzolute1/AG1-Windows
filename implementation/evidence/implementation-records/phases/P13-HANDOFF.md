# P13 handoff — explicit authority cutover

> Historical evidence, archived 2026-09-13. Status, commands, permissions and paths below describe their original checkpoint. For current work start with [implementation status](../../../README.md). Original bytes and old paths are preserved in the [relocation manifest](../manifest.json) and recovery archive.

Start P13 only when explicitly requested. The P12 user requested a commit and stop. This document does not authorize cutover. Require `implementation/evidence/P12/completion.json` with status `passed` and verify its complete artifact manifest, current original pins and tuned runtime fingerprints.

```sh
git status --short
git log -3 --oneline
python3 -c 'import sys; sys.path.insert(0,"tools"); from phase_gate import verify_dependencies; verify_dependencies("P13"); print("P12 artifacts verified")'
python3 -c 'import sys; sys.path.insert(0,"tools"); from common import verify_original; from runtime_gate import preservation; verify_original(); preservation(); print("Original pins and tuned runtime verified")'
```

Read `phases/P13-authority-cutover.md` as resolved through `planning/phase-graph.json`, common phase readings, P12 REPORT/completion, `product/clients/README.md`, `product/clients/COMPATIBILITY.md`, SDK README, and P10/P11 backend contracts. Earlier delivered snapshots remain immutable evidence; run current source checks separately.

The fixed-startup switch is `AG1_NATIVE_CONFIG=/absolute/private/host.json`. If absent, the existing TS worker remains the desktop authority. If present, the Tauri main window binds to `ag1_platform::client::Host`, accepts only explicitly marked isolated workspaces and uses private native state under the desktop's private directory. It cannot silently adopt an ordinary project. Native CLI uses its own explicit `--workspace`, `--state` and `--config` arguments. CLI/JSONL, standalone SDK and desktop share the same generated protocol and native host. The JSONL event/view loop and renderer do not own the native driver lifetime.

Native configuration admits exact provider/model definitions, optional process/compiler/browser toolchains, explicitly trusted extension helpers, native resource/package settings and environment credential bindings. It does not import the live user's profile/vault or claim live endpoint ownership. P13 must perform the planned journal/config migration and exclusive owner transfer before changing ordinary defaults. Preserve raw unset/false/zero profile fields and per-model binary/environment/argument precedence. Do not turn a successful readiness probe into an ownership grant.

P12 surfaces cover conversation/input/attachments, files and resource CAS, tasks/context/memory/artifacts, activity/recovery/usage, plans/questions, models/context conflicts, tree/fork/export, packages/extensions, target controls/problems/logs, owned previews and lazy terminal. Byte pages and immutable client-effect receipts retain full evidence and avoid replay on ambiguous acknowledgement. Preview windows have no AG1 command capability and are closed when their owned runtime grant expires. Refer to the explicit 85-row compatibility table for reference-only, removed and unsupported APIs; those are not desktop migration claims. P10 dynamic-rebind/DenyBridge and P11 judge/native-embedding boundaries remain in force.

P12 acceptance executes current native regressions, actual isolated CLI/JSONL, built SDK and component contracts, real WebKit preview privilege denial, matched P04 visual comparisons, 1,000-message scroll and a 30-second frozen renderer with resnapshot. The gate requires native controlled display p95 below 50 ms. Exact measurements and all source hashes live under `implementation/evidence/P12`. Fake provider timings are not live inference benchmarks. Linux private Xvfb/Chromium prerequisites retain the P11 tooling lock; other-OS and final long-duration/live/package acceptance remain P14.

L01–L08 are binding. Preserve these exact tuned servers and adjacent libraries:

- `/home/alex/Projects/llama-cpp-turboquant/build/bin/llama-server`
- `/home/alex/Projects/llama-cpp-qwen4exp-mtp-b10715/llama-server`
- `/home/alex/Projects/llama-cpp-qwen4exp/build/bin/llama-server`

`/home/alex/.ag1/agent/llamacpp-model-profiles.json` stays SHA-256 `aa02688c4d39ef73234f0ec381eed22dbe02463881626423049b75b5a12ad0d0`. Keep all GGUF shards/projectors, aliases, templates, sampling/cache/stop/speculation settings, model capabilities and ordered tiers. Never rebuild, upgrade, substitute stock llama.cpp, retune implicitly, evict an external server or stop by unowned PID/port. Cruzible remains read-only at `e30f0b4b238ee5618928bcd17b4571eb046e7038`, Dyad at `fb17039bfd2adcac7d74b9c1178a3a3524df35b6`.

Rollback keeps ordinary TS authority selectable only across a quiescent boundary. Retain native receipts, journals, artifact custody and unresolved intent evidence. Do not replay uncertain client effects or overwrite user recovery evidence.
