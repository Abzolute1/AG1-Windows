# Preserve the user's tuned llama.cpp setup

The user explicitly required reuse of the existing llama.cpp setup before roadmap planning on 2026-09-04. This requirement controls every implementation phase. The final Rust core supervises and calls the existing backends; it does not replace the inference engine. Backend modernization and tuning experiments are outside this migration unless separately authorized by the user.

## Observed configuration

Read-only inspection found `/home/alex/.ag1/agent/llamacpp-model-profiles.json`, schema version 1, SHA-256 `aa02688c4d39ef73234f0ec381eed22dbe02463881626423049b75b5a12ad0d0`. The exact non-secret profiles and fingerprints of 57 unique executable/adjacent shared-library files are recorded in [runtime-lock evidence](evidence/llama-runtime-lock.json). No model was launched, stopped, queried, rebuilt or tuned.

| Configured model | Required existing executable | Important profile distinctions |
|---|---|---|
| `Qwen3.6-35B-A3B-UD-Q4_K_XL` | `/home/alex/Projects/llama-cpp-turboquant/build/bin/llama-server` | 90,000 context, 65,536 output ceiling, MoE CPU 20, q8_0 K/V, fit off, GPU layers 99, batch/ubatch 256/128, decode/batch threads 12/12, load-mode none, slot similarity flag omitted, priority 2, `--reasoning-preserve` |
| `Qwen3.8-Flash-Next-AD-3.84bpw-IQ4_XS-M64` | `/home/alex/Projects/llama-cpp-qwen4exp-mtp-b10715/llama-server` | 90,000 context, 65,536 output ceiling, MoE CPU 42, q8_0 K/V, fit off with target 1024, GPU layers auto, base batch/ubatch 1024/512; context tier at ≤90,000 overrides ubatch to 1024; threads 12/24, slot prompt similarity flag, priority omitted, metrics/verbosity |
| `Qwen3.8-27B-UD-Q4_K_S` | `/home/alex/Projects/llama-cpp-qwen4exp/build/bin/llama-server` | 65,536 context, 32,768 output ceiling, q8_0 K/V, draft-mtp with max draft 3, fit on, GPU layers auto, batch/ubatch 1024/512, threads 12/24, slot prompt similarity flag, priority omitted, reasoning-preserve/metrics/verbosity |

All three profiles set unified/offloaded KV and flash attention on. These are recorded settings, not independently verified effective runtime values. In particular, absent fields must retain their resolved meaning; do not invent a default library path for the 27B profile or infer MTP activation from the Flash backend directory name.

The process environment points `AG1_LLAMA_SERVER` and `AG1_LLAMA_LIB_PATH` at the turboquant build. **Model-specific `serverPath` wins over that fallback.** A desktop launcher may inherit different environment from a terminal, so import explicit runtime selection and compare its resolution instead of relying on GUI PATH.

The turboquant source checkout is clean at `bd1bf025fc55ffa1fcb2ba6d8bb8805f35671d1f`. The qwen4exp source checkout is at `cc83d7b4824f73cfdda4dfbb47ee39804f71b328` with a pre-existing modified `AGENTS.md`; preserve it. The `mtp-b10715` directory has no local Git metadata. Directory names and source HEAD alone do not prove how a binary was built. Hash the executable **and** implementation libraries: the small launcher executable is not the entire backend.

## Preservation invariants

| ID | Required behavior | Acceptance evidence |
|---|---|---|
| L01 | Each model retains its own pinned executable/build and loader dependencies. Missing/mismatched identity is a visible conflict; no stock fallback, upgrade or automatic rebuild. | Three-profile selection tests; tampered binary/library and missing path tests; package installer cannot overwrite runtime directories. |
| L02 | Preserve profile fields, unknown/raw source bytes and effective precedence: explicit context actions, model defaults, environment, VRAM profile, hardware fallback, context tiers and ordered extraArgs. | TS capture versus Rust launch-plan comparison at tier boundaries, absent/false/zero distinctions and all three real profiles. |
| L03 | Preserve GGUF shards, projector selection, alias, model context/output limits, chat template, reasoning/tool formats, request sampling/stop/cache behavior and speculative decoding configuration. | Frozen model identity and request-body fixtures; tool/reasoning/multimodal canaries and exact-copy workload; compare quality as well as speed. |
| L04 | No migration-triggered tuning or rewriting user profiles. Keep known settings by default; existing opt-in tuning capability must remain available as a separately requested action with reversible profile versions. | Import/open/model switch cause zero tune calls or profile writes; explicit tuning request is distinctly recorded and cannot affect another model. |
| L05 | Exactly one owner admits local generation; Rust shadow tests and the TS bridge do not compete against the same endpoint. | Endpoint lease, ignored-abort reconciliation and cutover tests; no double permit acquisition. |
| L06 | Readiness/model identity is separate from permission to stop a process. Observe externally running services; do not adopt/kill by port or PID alone. | Foreign listener, reused PID, owned child, host restart and explicit quit tests. Existing user services are left running unless explicit ownership/control is established. |
| L07 | Preserve original runtime directories, user configuration and weights; import into a new private destination and retain a verifiable rollback. | Before/after hashes; importer interruption test; no package update writes to the original runtime; local secrets never enter reports. |
| L08 | Judge migration with the same backend/config/model/hardware and equivalent prompts/cache state. A faster but behaviorally different backend is not migration success. | Sequential TS/Rust comparisons per model: quality, tool execution, TTFT, prefill/decode, context capacity, VRAM/RAM and optional draft acceptance. |

## Implementation boundaries and remaining evidence

`packages/coding-agent/src/core/llamacpp-runtime.ts` is authoritative for existing resolution and argv construction: `resolveLlamaCppModelProfile`, `resolveLlamaCppContextBatchTier`, `buildLlamaCppServerArgs`, `ensureLlamaCppServer`, `buildLlamaCppProviderConfig`. Profiles override binary discovery; extra arguments are appended in order. `ensureLlamaCppServer` merges profile environment and conditionally prepends the selected libPath. `priority: false`, `slotSimilarityFlag: none`, absent settings and explicit false values matter. Speculation currently suppresses projector selection. Preserve behavior through fixtures, not a generic Rust launcher.

The observed lock is intentionally incomplete as an execution manifest: it does not hash GGUF/projectors, resolve dynamic system/CUDA loader dependencies, capture private inherited environment, establish build flags or identify a currently running server. P01 completes these facts locally without changing them; unknown provenance stays unknown. Full private launch material lives outside versioned reports. Redacted reports must retain hashes/references sufficient to detect changes without exposing credentials.

P04 first uses the TS launcher/resolver behind a guard that honors this lock and disables unsolicited tuning. P06 builds the Rust launch-plan adapter against captured TS fixtures and uses fake processes. P13 transfers control only after the TS runtime is quiescent, and validates or deliberately restarts only an owned process with the same backend/config. An existing external server remains external. P14 closes live model and packaging gates.

If an OS cannot run the existing Linux builds, report that platform's tuned-local-runtime support as unavailable. Do not substitute a generic macOS/Windows llama.cpp binary and call it the preserved setup. The shell can still be tested there with fake or explicitly configured external endpoints. A new platform-specific backend build is a separate authorized compatibility project.
