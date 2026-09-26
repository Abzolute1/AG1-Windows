# Cache policy, placement and diagnostic scope

Read-only source review of original `bd1bf025fc55ffa1fcb2ba6d8bb8805f35671d1f`. No cache source patch, compilation, hash, benchmark or model test performed. Parent owns the ongoing cache configuration measurements.

## Host pinning, repack and prefill are separate choices

`common/arg.cpp:841-844` makes numeric/on `--moe-cache` disable extra CPU buffer types (`no_extra_bufts=true`). It does not set `no_host`. `src/llama-model.cpp:930-941` puts CUDA_Host before CPU extra/repack buffers. Thus a canonical CUDA_Host baseline retains that buffer selection with explicit numeric cache, and does not lose its large-batch GPU offload simply because repacking was disabled.

`--no-host` changes both pinning availability and which later buffer type wins selection. On this AVX2 build, Q4_K can select CPU_REPACK; Q5_K/Q6_K cannot use the ARM-only repack choices and fall back to ordinary CPU memory. CPU_REPACK has no is_host callback (`ggml-cpu/repack.cpp:4829`), so `ggml_backend_buffer_is_host` is false. The scheduler's higher-priority GPU large-batch offload gate explicitly requires host storage (`ggml-backend.cpp:1115-1121`). Those repacked Q4 weights stay in the CPU execution path. Canonical pageable CPU weights may still offload, with slower pageable transfers. Consequently the parent's earlier --no-host timing cannot isolate the slowdown to either repacking or pinning without per-buffer placement and transfer evidence.

`--moe-cache auto` does not trigger the argument parser's forced no-extra-buffer policy. `GGML_CUDA_MOE_CACHE_BUDGET_MB` is read in provider configuration, then retained unless a positive explicit API/CLI budget replaces it (`ggml-cuda/moe-cache.cu:900-916`). This permits auto plus an environment budget while preserving ordinary buffer selection. It does not make already-repacked tensors cache-compatible: model eligibility and CPU cache entry require host buffers, excluding CPU_REPACK. Canonical CUDA_Host tensors remain eligible.

Provider cache batches are capped at eight tokens (`moe-cache.cu:52,1850`); larger prompt batches use the original scheduler/backend route. Cache pool memory and reserve pressure can still affect prefill capacity/allocations. Record resolved policy and actual pools, not merely requested MiB. AUTO and ON have different minimum slab/admission policy; `query_config` resolves explicit AUTO after reading environment defaults.

## Lifetime heat protection can resist workload changes

At `moe-cache.cu:2225-2256`, eviction scans LRU order, skips pinned/readers-active slots, and prefers the first slot whose `uses <= hot_uses`. Only when no such cold candidate exists does it evict the oldest unpinned hot slot. `uses` increases on ordinary hits at 2330 and on fused paths at 2928-2930/2962/2977; it resets when a slot is reset at 767. No decaying/aging update was found in these use/reset paths.

Therefore formerly hot entries can remain protected while a small colder region repeatedly churns after a prompt-domain shift. Parent reports review-to-JSON hit-rate deterioration and very few heat evictions relative to total evictions; that is consistent with this mechanism, but aggregate counters alone cannot identify stale entries, layers or pool imbalance.

`GGML_CUDA_MOE_CACHE_HOT_USES=1073741824` is accepted (parser maximum is `1 << 30`, lines 590-591). For practical run lengths, every slot is then below the hot threshold, making victim selection ordinary LRU among unpinned slots. Admission/readmission thresholds, fill queue limits, background fills, pool partitions and CPU overlap remain unchanged. Test that configuration before implementing aging. An eventual aging change would need a defined epoch/window and careful lock/counter semantics; none is implemented or recommended before the LRU evidence arrives.

## Narrow external instrumentation is feasible, with limits

The provider's begin/plan implementations are static callbacks, not exported `ggml_moe_cache_begin` functions. The exported `ggml_moe_cache_register` accepts a callback table and copies it by value (`ggml-backend.cpp:27-43`; API in `ggml-backend-moe-cache.h`). An LD_PRELOAD diagnostic could intercept registration at process startup, retain the original CUDA provider table and forward a copied table with wrappers. No CUDA kernel rebuild is necessary. It must preserve unknown/other providers, every callback signature, all return values, pointer ownership and error semantics; do not replace a live session's callback table after creation.

Useful wrappers and observations:

- `begin`: tensor name, immutable host-base identity, bytes per expert, input/output dimensions, quant type, token count and routed row count; associate a non-null opaque node with copied metadata. A null result means full CPU fallback or an ineligible/dormant/contended case, not necessarily a residency miss.
- `plan`: expert IDs, nonnegative post-policy slot indices and returned hit count. Track per-layer/quant-type routed demand and effective GPU eligibility. CPU overlap intentionally removes otherwise resident rows at `moe-cache.cu:2340-2355`, so negative output indices are not unconditionally true cache misses.
- `dispatch` and `collect`: retain success/failure to distinguish intended offload from actual CPU fallback or recomputation. A planned row is not proven GPU-served until both succeed.
- `fused_begin`: both up/gate tensor descriptors, expert IDs and returned hit mask. This gives effective jointly serviced pairs, not each member's individual residency. A pair can fail because either member is missing, overlap policy, contention or dispatch failure. The internal separate up/gate lookup arrays at `moe-cache.cu:2881-2910` are not exposed. Exact per-member residency/eviction attribution would require a narrow provider-side stats hook, not unsafe decoding of opaque private structs.
- `end`: remove opaque-node bookkeeping exactly once; never retain a model/tensor pointer beyond its lifetime. For successful fused nodes, collect/end are shared with the ordinary path.
- Session enter/leave/destroy can tag session identity and flush aggregated statistics. Registration refresh/unregister and multiple providers require explicit handling; scope the first experiment to the actual single CUDA provider and single model run.

No API callback labels HTTP requests or distinguishes a one-token prefill from decode. Use monotonic timestamps aligned with the parent's exact request/phase timeline, or a separately provided phase epoch sampled outside the hot callbacks. `n_tokens > 1` is a shape hint, not an authoritative phase label. Never read prompt text or poll files on every expert row.

Prefer in-memory counters grouped by session/phase/layer/tensor or fused pair, including rows, estimated CPU bytes, effective GPU rows, bypasses, dispatch/collect failures and expert demand histograms. Preallocate or reuse metadata and emit a few summary records per phase; per-row logging would distort the workload. Treat instrumented runs as diagnostics, not final timing evidence. `OVERLAP_CPU_ROWS=0` is a useful separate diagnostic to remove intentional CPU work from the negative-slot count.

## Using diagnostics for placement

Rank residual CPU work by effective routed rows times expert bytes and measured type/shape CPU cost, not miss rate alone. Fused gate/up failures should count both CPU matrices where the CPU fallback really executes. Include each entire layer's permanent GPU weight cost and the changed cache budget when estimating a replacement for "last N layers". Compare across reviews, JSON and code; a placement optimized only for review traffic can reproduce the observed domain-shift problem. Re-test chosen static placement with the same cache warmup/policy and VRAM/context budget. No placement improvement is established by this review.

## Parent-reported LRU ablation checkpoint

Parent subsequently reported that CPU40 plus 9216 MiB cache, with the same configuration except `HOT_USES=1<<30`, restored structured JSON decode from approximately 43 to 85 tok/s, medium review from approximately 79 to 82, and code from approximately 82 to 86; exact-output/API checks passed. These are parent-run checkpoint numbers, not new measurements by this source-review agent, and repeated final measurements remain the authority for accepted gains.

This configuration-only ablation strongly supports stale-hot protection as the workload-transition sensitivity. No source arithmetic, stored quantized weight bytes, quantizer or KV precision changed. Eviction changes residency and can change which existing CPU/GPU kernel executes a routed row, so it is not a universal proof of bitwise identity; the matched output checks remain necessary. Prefer this proven configuration direction to an unmeasured aging rewrite.

The diagnostic source/build artifacts now exist under `/home/alex/Projects/llama-perf-20260914/cache-routing-profile/`. Source preparation and plan-only validation are complete; this review agent has not compiled or run them.
