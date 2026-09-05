# P06 — Providers, scheduler and preserved llama.cpp supervision

## Objective and reading

Port model orchestration while retaining the user's exact tuned inference backends. Read common contract, LLAMA_CPP_PRESERVATION and its lock, DECISIONS D04, AG1 `llamacpp-runtime.ts`, runtime/VRAM tests, AI provider adapters, auth/capability/thinking/vision, scheduler and fallback tests. Scope A49–A58; I16,I18,I21,I25,I29; all L01–L08.

## Entry and allowed work

Require P03 process/network/secret ports, P05 settings storage, P02 protocol and P01 launch/provider fixtures. Implement core provider/scheduler modules and platform HTTP/local-runtime adapters. Use fake HTTP/process/GPU interfaces and private fixture config. No backend source changes, recompilation, binary installation, tuning, profile rewrite or live service eviction.

## Work and authority

Separate pure `resolveLaunchPlan` from `startOwnedRuntime`, `observeExternalRuntime` and `requestGeneration` APIs. A launch plan includes model/profile/build fingerprints, ordered executable/argv, effective environment references, loader paths, requested/resident limits, projector/speculation and selected override sources. Compare against P01 TS captures before any process launch. Preserve version-specific flags; unknown/missing pinned binary fails visibly. Do not probe `--help` and silently drop an unsupported flag.

Test all three real profiles: profile binary overrides turboquant environment fallback; Flash 90k context tier overrides ubatch to 1024; false priority omits the priority flag; `slotSimilarityFlag: "none"` omits similarity flags, while an unspecified value retains the reference default; 27B retains draft-mtp/max-draft3 and fit-on; base library inheritance remains exact. Cover /set-context, profile/environment/VRAM precedence, model switch, split GGUF/projector selection, request-side thinking/tool/template/sampling/cache/stop behavior. Do not force all models onto one “fast” profile. No auto-tune on migration/open/switch; explicit tuning capability is separately invoked and versioned.

Port every active provider family from the P01 child ledger, including credentials/OAuth refresh, text/reasoning/tool fragments, usage/cache/cost, error/stop/timeout and compatibility fallback. Rust handles transport/domain normalization; external OpenAI-compatible endpoints do not require owned llama processes. Secrets remain host-only. Unsupported capabilities/errors are explicit.

The pinned `packages/ai/src/providers/register-builtins.ts:registerBuiltInApiProviders` registers ten API families: `anthropic-messages`, `openai-completions`, `mistral-conversations`, `openai-responses`, `azure-openai-responses`, `openai-codex-responses`, `google-generative-ai`, `google-gemini-cli`, `google-vertex`, `bedrock-converse-stream`. Give each its own stream/error/auth fixture group. Dynamic extension providers are additional P09 integration cases. This list is a repository inventory, not a claim about current external service availability.

Implement bounded endpoint queues preserving FIFO and default local1/remote3 capacity. A permit lasts through terminal settlement. Ignored abort quarantines/reconciles the endpoint before new local generation. Isolated summary/episode calls acquire exactly one permit. Model lifecycle is inspect/ready/loading/failed/stopping with actual identity checks; external observation is not adoption. Restore process ownership from verified tokens, never persisted PID/port alone.

## Tests and commands

Create core `provider_contract` and platform `llama_contract`. Use fragmented fake SSE, error before first token/mid-tool/after terminal, slow prefill, OAuth expiry, 403 fallback, ignored cancellation and fair FIFO queue fixtures. Fake runtime cases cover wrong model/context, missing library, changed executable hash, port conflict, load failure, restart, quit, stale PID, no tuning and exact argv/environment resolution. Confirm both model profiles and original binary hashes remain unchanged.

```sh
cargo test --manifest-path product/Cargo.toml -p ag1-core --test provider_contract
cargo test --manifest-path product/Cargo.toml -p ag1-platform --test llama_contract
python3 tools/phase_gate.py P06
```

No frontend extraction. Record HTTP/auth dependency notices; no blanket clearance of model weights/backend bundles. Live canaries are a separate mode, only with a free explicitly designated endpoint; no overlap with TS bridge. If unavailable, mark live checks pending for P13/P14, not passed.

## Measurement and completion

Measure normalization/queue memory/latency using identical fake rates and payloads. When live access is available, run exact-copy, reasoning and tool roundtrip sequentially per model with locked prompt/cache/sampling; record TTFT/prefill/decode/VRAM and quality, not just tokens/sec. Exit requires complete provider child fixture coverage, exact model launch plans, safe lifecycle and no tuning/fallback. Unknown loader/weight identity blocks a live equivalence claim.

## Rollback and handoff

TS remains scheduler/runtime authority for the usable desktop. Stop only owned fixture resources; discard new test settings. P08 receives isolated model-call ports, P10 a normalized provider/scheduler, P13 an exact runtime-control transfer contract. Original llama binaries, libraries and profiles remain untouched.
