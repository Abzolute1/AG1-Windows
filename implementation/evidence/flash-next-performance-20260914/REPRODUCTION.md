# Reproducing this investigation

> Sanitized technical document. Personal paths, machine identifiers and application-specific records have been removed. Measurements are historical and hardware-dependent; they are not portable performance guarantees. Referenced raw evidence and source bundles have not been sanitized or included. This document alone is not a complete executable reproduction package.

Discovery is closed. This protocol records the investigation and the fixed final comparison; the final recommendation belongs in REPORT.md. Historical controllers are audit artifacts, not a queue to restart.

## Inputs and isolation

The working scripts are in `<EXPERIMENT_ROOT>`. `original-launch.json` in this evidence directory is the captured baseline command and environment. `model-materials-before.json` identifies the preserved model shards; `model-layout.json` records actual tensor types and locations. `protected-before.json` records protected runtime/configuration files. Keep these historical records unchanged.

For another model or another session, create fresh experiment and evidence roots, update script root constants, and capture a new baseline from the actual application's launch. Do not reuse the current model identity, runtime arguments, quantization assumptions, source overlays or result directories blindly. A filename containing IQ4_XS does not prove the tensors use IQ4_XS.

Inspect architecture before interpreting “n-gram”: Flash-Next's learned PLE table and optional runtime speculative drafting are separate systems. Learned table lookups have no speculative acceptance rate. Follow the source paths for the specific architecture and record the model's own metadata.

## Baseline protocol

1. Identify the exact app-owned server executable, loaded libraries, source/build revision, arguments, relevant environment and sealed configuration. Do not log unrelated inherited credentials.
2. Hash protected files/model shards outside timed inference. Never overwrite or re-quantize the model. Budget disk space before source archives, builds or model downloads.
3. Run one GPU model process at a time. Do not overlap compilation, unit-test GPU use, model hashing or heavy copying with timed inference. A scheduler must wait for the actual preceding process to exit; an old completion marker is not sufficient. The stale-marker overlap in `invalid-runs.json` is excluded evidence.
4. Freeze tokenized prompts and expected answers before A/B comparisons. Use separate workload classes, fixed greedy sampling for execution comparisons, unchanged context and target KV precision, and explicitly fresh prompt caches. Treat OS page-cache warmth separately from the model's KV/prompt cache.
5. Use `bench.py CONFIG.json` to launch an isolated server on its dedicated loopback port, record model-load/health latency, capture actual mappings and arguments, stream requests, sample resources and clean up only its owned process group. Existing result directories are rejected. Repeated real-model requests are the primary evidence; microbenchmarks identify candidates.

The runner records request bodies, raw SSE events, server timings, response text, TTFT/wall latency, CPU/RAM/GPU/PCIe/disk telemetry and smaps snapshots. `cache_prompt=false` measures fresh prefill. The conversation fixture explicitly warms and appends a persistent context. `prepare_long` freezes a complete-record notebook around 83,000 tokens using the actual tokenizer, within the unchanged 90,000-token requested context.

## Source and build experiments

Each build command manifest gives the exact compiler arguments, source root and output. Runtime-only experiments preserve all other baseline flags. Source changes have isolated patches and bundles. Some bundles reuse the exact compatible release CUDA backend; full the target GPU architecture builds test that choice separately. Never mix libraries across ABI-changing patches, notably the expert-pool context-parameter extension. Verify loaded mappings on real model runs.

The PLE and masked-warp experiments use source overlays: rebuild the changed translation unit with the exact recorded build command, then relink against read-only objects from that same base build into a new bundle. Their scripts redirect object, dependency-file and library outputs into the overlay; they do not rebuild in place. The original build objects must still exist for this reproduction method. The patched bundle should then stand alone at runtime, verified through mappings.

## Interpretation

For controlled load-time comparisons, `prewarm_weights=true` reads all non-PLE primary-model shards before starting the model-load timer. The preparation bytes/time are separately recorded in `prewarm.json`. This defines a warm file-cache starting condition; it is not a cold-start result. Continue inspecting actual process disk reads during load: allocating a second large pinned copy can evict the source pages even from an initially warm cache. The PLE table remains demand-paged. The runner refuses this prewarm mode for alternate model paths unless their manifest is explicitly adapted.

The executed final command is `python final-validation-suite.py closeout-candidate.json --focused-closeout`. It uses two reversed full-answer orders (original/config-only/candidate, then candidate/config-only/original), followed by one original/candidate pair at 84,859 prompt tokens. Six earlier long-context runs provide additional context. This fixed eight-process scope supersedes the runner’s broader default. Use fresh names if reproducing; existing evidence must not be overwritten.

The private-mapping implementation requires build-specific switches. Obtain their exact names from the matching source patch; no portable stock-runtime flag is implied.

After all inference exits, `final-preservation-audit.py` rechecks protected runtime/configuration files, all model and MTP materials, source states and installed package contents. `archive-final-reproduction.py` archives scripts, patches and build commands without private credentials or model weights. These scripts refuse to overwrite final evidence. Read the final report and manifests before using any historical command.
