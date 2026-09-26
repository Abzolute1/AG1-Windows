# Read-only llama.cpp source review

Source tree: `/home/alex/Projects/llama-cpp-turboquant`

Reviewed revision: `bd1bf025fc55ffa1fcb2ba6d8bb8805f35671d1f` (TurboQuant fork, original build 10530). All source references below are relative to that tree and revision. No original source, binaries, configuration or models were modified. This review did not run builds or GPU workloads.

## Confirmed source behavior

- CPU waiting can inflate utilization without representing useful expert computation. **Correction:** the initial attribution to `--poll 100` was wrong for this installed build. The polling function at `ggml/src/ggml-cpu/ggml-cpu.c:3675` and its `1024UL * 128 * poll` rounds are inside `#ifndef GGML_USE_OPENMP` (lines 3643-3779). The preserved CMake cache has `GGML_OPENMP=ON` and `GGML_OPENMP_ENABLED=ON` (evidence `original-build-cache.txt:803`, `:1526`); this build executes `#pragma omp parallel num_threads(n_threads)` at `ggml-cpu.c:3888` instead. Its relevant worker waiting is managed by the linked libgomp runtime. Thus changing `--poll` was a no-op for this worker path, and the earlier proposed poll comparison does not establish anything about spin overhead. Approximately 1160% process CPU utilization still does not prove twelve cores are doing useful mathematics; test OpenMP runtime waiting controls such as `GOMP_SPINCOUNT` and `OMP_WAIT_POLICY` instead, measuring latency and throughput together.
- CUDA graphs are disabled by the presence of `GGML_CUDA_DISABLE_GRAPHS`, regardless of its value (`ggml/src/ggml-cuda/common.cuh:1263`). Setting it to `0` still disables graphs; a graphs-enabled control must leave it unset.
- `GGML_CUDA_GRAPH_OPT=1` is an opt-in graph transformation (`ggml/src/ggml-cuda/ggml-cuda.cu:4371`). It requires CUDA graphs and exactly one CUDA device (line 4383). Candidate fan-out counting is limited to nodes with at most one row (line 4421), exactly three branches (lines 4438-4439), and root names containing `attn_norm` (line 4450). The source comment describes Q/K/V concurrency and explicitly says other uses have not been tested (line 4427). Dependency checks include direct inputs and shared view sources. Branches run on separate streams and rejoin through CUDA events; the original branch ordering is restored to retain within-stream fusion.
- Fused autoregressive and chunked Gated Delta Net operations already default to enabled (`src/llama-context.cpp:268`). Backend support is resolved once and `auto_fgdn` is then cleared (line 585). Thus upstream removal of nonfused fallback paths is not, by itself, evidence that this baseline leaves GDN unfused during steady-state inference.
- MoE cache mode defaults to `auto` (`common/common.h:459`). The CLI describes auto as preserving weight repacking, while `on` or an explicit MiB budget operates without weight repacking (`common/arg.cpp:2702`). Explicit activation is consequently a cache-versus-CPU-kernel tradeoff, not a free additional optimization.
- The cache implementation limits batches to eight tokens (`ggml/src/ggml-cuda/moe-cache.cu:52`) and routed rows to 64 (`ggml/src/ggml-cpu/ggml-cpu.c:1630`). Default device reserve is 3072 MiB (`moe-cache.cu:144`), and auto mode requires a minimum 1 GiB slab (`moe-cache.cu:54`, `:522`). Eligibility also depends on host expert tensor shape, size and buffer type (`src/llama-context.cpp:608`). These constraints make it chiefly a decode experiment; limited remaining VRAM may prevent it activating.
- CUDA's backend-to-backend asynchronous tensor copy rejects either endpoint being a non-CUDA backend (`ggml/src/ggml-cuda/ggml-cuda.cu:2581`). The generic host/device fallback uses buffer set/get operations; CUDA buffer set/get enqueue on `cudaStreamPerThread` and synchronize that stream (`ggml-cuda.cu:855`, `:859`). This establishes a plausible source of repeated transfer/synchronization overhead, but its share of runtime remains unmeasured by this review.

## Recommended profiling experiment

Use a separate original-revision source copy. Instrument `ggml/src/ggml-backend.cpp` without changing execution order or inserting synchronization:

1. Count scheduler graph calls, backend splits and input-copy edges in `ggml_backend_sched_compute_splits` around line 1786.
2. Time the existing input-copy section and `ggml_backend_graph_compute_async` call around line 1916, grouped by backend. CPU graph-compute elapsed time represents CPU execution; CUDA graph-compute elapsed time primarily represents enqueue/graph handling, not completed GPU execution. Label these distinctly.
3. Count and time existing waits at lines 1800, 1802, 1810, 1825, 1844, 1902, 1904 and 1906. Keep overlapping/nested aggregates separate to avoid double counting.
4. Accumulate ordinary full-copy bytes by CPU-to-CUDA and CUDA-to-CPU direction. Count actual selected-expert bytes inside the `copy_experts` lambda around line 1864, including its padding, instead of counting the full weight tensor.
5. Emit aggregate records per graph or at shutdown, preferably gated by an environment variable. Avoid per-node logging in performance runs. Compare instrumented versus uninstrumented execution to quantify measurement overhead.

`GGML_SCHED_DEBUG=2` plus sufficiently verbose logging (`-lv 4`) can first show split backends, inputs, tensor names and sizes without a patch (`ggml-backend.cpp:1142`, `:1583`). It supplies structural evidence, not timing. Run it separately from throughput measurements because verbose output can perturb results.

## Candidate source experiment: duplicate destination wait

Confirmed structure: in the no-event path, the destination split backend is synchronized before considering an input copy (`ggml-backend.cpp:1810`). If the generic asynchronous-copy attempt is unavailable or fails, the fallback synchronizes the source and then synchronizes the destination again (`:1901` through `:1908`).

Hypothesis: some no-event host/device copy edges could omit the second destination wait and reduce repeated host CUDA API overhead. The expected benefit may be small; first measure the number and elapsed cost of these calls. This is not an established optimization or a measured improvement.

Required semantic guard for any isolated patch:

- Apply only to the ordinary fallback branch, not the selected-expert-copy path.
- Record that the earlier destination synchronization actually occurred for this exact input and backend.
- Omit the second wait only when no operation has been submitted to that destination between the first wait and the fallback, including through an attempted backend copy. A failed asynchronous-copy API must not have queued work; establish that for the selected backend or restrict the fast path to a proven unsupported endpoint case.
- Preserve the source synchronization and the actual tensor copy. Preserve all event-path waits and ordering.
- Do not infer asynchronous safety from tensor size or from the baseline's single-slot setting alone.

The supplied CUDA asynchronous-copy implementation returns immediately for CPU endpoints before submitting work (`ggml-cuda.cu:2585`), providing a narrowly provable case. Generalizing across other backends needs separate proof.

## Ranked hypotheses to validate

1. Larger prefill batches/micro-batches amortize graph and expert-transfer overhead. Parent-run measurements already made this a promising configuration candidate; this source review itself supplies no benchmark claim.
2. The original lower-`--poll` hypothesis is disproven for this OpenMP build because that polling implementation is not compiled into its worker path. Test libgomp waiting controls instead, or an isolated same-revision CPU backend built without OpenMP. Measure throughput, latency and utilization together; neither alternative is a proven improvement.
3. `GGML_CUDA_GRAPH_OPT=1` may improve single-token full-attention work. It does not enable general GDN concurrency or CPU/GPU overlap. Do not broaden its three-branch restrictions without dependency and aliasing proof.
4. Explicit MoE cache budgets may accelerate repeated hot experts if their benefit exceeds lost CPU repacking and cache overhead, and sufficient VRAM remains. Check resolved activation and actual hit statistics before attributing a result to caching.
5. Scheduler profiling may justify the duplicate-wait experiment above. Batching multiple host/device input transfers into one stream and wait is a more invasive future direction requiring input-lifetime and stream-order analysis; it is not recommended for implementation without profiling evidence.

No optimization is accepted on source plausibility alone. Final recommendations must use the parent investigation's repeated Qwen3.6-35B-A3B real-inference measurements and correctness checks.

## Side-by-side upstream source comparison

Compared the separate source trees `original-experimental` at `bd1bf025fc55ffa1fcb2ba6d8bb8805f35671d1f` and `upstream-latest` at `ad6c66839af3c5646fba8c6c2e2087a1e4e38948`, both under `/home/alex/Projects/llama-perf-20260914`. This comparison used targeted source reads/diffs only. References in this section refer to the upstream revision unless marked original. Differences include removal of fork-specific features; they are not all new upstream optimizations.

**CLI compatibility:** upstream accepts `--load-mode none` (`common/arg.cpp:2687`), `--n-cpu-moe` with first-N-layer placement (`:2763`), `--no-webui` (`:3457`) and `--reasoning-preserve` (`:3722`). Its cache-type list includes q8_0 (`:308`). These original settings need no translation. The fork's `--moe-cache` is absent, so remove that flag from comparisons derived from explicit cache experiments; it was not part of the original launch. Upstream Flash Attention defaults include the q8_0/q8_0 kernel pair (`ggml/CMakeLists.txt:207`), so enabling all quantization combinations is unnecessary for the primary baseline. Source compatibility still requires actual build/startup validation.

**Relevant changes justifying a benchmark:**

- Upstream adds a CUDA fused MoE weighted reduction: matching begins at `ggml/src/ggml-cuda/ggml-cuda.cu:3037`, and dispatch at `:3442` checks fusion memory ranges before replacing the matched sequence. `moe-weighted-reduction.cu:4` performs expert weighting and accumulation in one kernel. It accepts 2 through 15 active experts; the target's eight fit that bound. A matching CUDA-resident graph could avoid intermediate writes and kernel launches. Source inspection alone does not prove that every target layer matches or establish a throughput improvement.
- Qwen35MoE full-attention projection construction now uses the shared `build_qkv` helper (`src/models/qwen35moe.cpp:290`). The helper supports fused QKV weights and otherwise retains separate projections (`src/llama-graph.cpp:1633`). The presence of that helper does not imply that this existing GGUF's separate weights are fused automatically.
- Qwen35MoE GDN normalization changes at `src/models/qwen35moe.cpp:453`. The new helper (`src/models/models.h:14`, referencing [upstream PR 28068](https://github.com/ggml-org/llama.cpp/pull/28068)) computes `scale(rms_norm(x, eps/n), 1/sqrt(n))`, equivalent in real arithmetic to `x/sqrt(sum(x^2)+eps)`. The original `ggml_l2_norm` CPU implementation uses `x/max(sqrt(sum(x^2)),eps)` (`original ggml/src/ggml-cpu/ops.cpp:4220`). This is a numerical behavior change, not merely a scheduling rewrite. Exact sampled output equality across revisions is therefore not a sufficient correctness criterion; use task correctness and output-quality checks alongside performance. Model weights and quantization remain identical.
- Upstream scheduler adds a wait when switching backends into a split with no inputs, because allocator regions can be reused across splits (`ggml/src/ggml-backend.cpp:1657`), and records events for every split (`:1834`). This is an ordering safeguard that may add work in affected graphs. It is not a speed optimization and should not be removed to improve benchmark numbers.

**Changes not sufficient to predict a gain on this machine:**

- Much of the MMVQ diff adds DGX Spark-only prefetching and tuning (`ggml/src/ggml-cuda/mmvq.cu:8`), explicitly disabled on higher-bandwidth GPUs. New Ada dispatch cutovers at `:322` concern Q2_K and Q3_K, while this model's significant quantized weights are Q4_K, Q5_K, Q6_K and q8_0. Do not attribute expected RTX4080 gains to the Spark prefetch work.
- MMQ's expert-average tile selection adjustment is limited to RDNA3/RDNA4 (`ggml/src/ggml-cuda/mmq.cu:247`); its old-Pascal dispatch adjustment likewise does not target Ada.
- The targeted CPU repack diff contains only a VXE capability case (`ggml/src/ggml-cpu/repack.cpp:4589`). It does not establish a new Zen3 repack optimization.
- Sparse Flash Attention support requires a positive sparse `n_kv_max` and matching masks (`ggml/src/ggml-cuda/fattn.cu:114`). Its presence is not evidence that ordinary Qwen3.6 causal attention automatically becomes sparse. The q8/q8 Ada decode dispatch still selects vector attention at very small query batches (`:621`).
- Upstream lacks the fork's TurboQuant cache formats and adaptive CPU-expert VRAM cache. q8/q8 baseline compatibility does not establish compatibility with every possible AG1 profile or TurboQuant experiment.

**Recommendation:** benchmark this pinned upstream revision side-by-side with the same CUDA89 Release architecture, original model, q8/q8 cache, context, placement and prompt token IDs. The fused MoE reduction and relevant graph changes supply concrete reasons to test it, but no gain is accepted from this source comparison. Include quality checks that account for the normalization correction, and compare against both the preserved original configuration and the best proven configuration-only candidate.

## OpenMP correction and separate CPU-backend experiment

The conditional compilation correction above was verified by reading the guarded worker code and the preserved build cache. It preserves the initial hypothesis as a rejected source interpretation rather than presenting the `--poll` experiment as a meaningful wait-policy intervention.

`ggml/src/ggml-cpu/CMakeLists.txt:75` adds `GGML_USE_OPENMP` as a private compile definition to the CPU backend target and links that target against OpenMP. With OpenMP enabled, `ggml-cpu.c:3817` assigns worker masks and `:3888` enters an OpenMP parallel region for graph execution. The no-OpenMP branch instead creates persistent worker threads (`:3824`) and uses the custom polling/condition-variable path. Both invoke the same graph computation routine, but scheduling, startup, affinity assignment and barriers can differ. In particular, the persistent-worker branch assigns worker masks to secondary threads before the main thread, unlike the OpenMP branch; preserve the eligible physical-core set and record that assignment-order difference.

An experimental CPU-only backend build at the exact same revision with `GGML_OPENMP=OFF` is technically justified: it tests whether repeated OpenMP parallel-region/waiting overhead matters for the many small CPU expert graphs. The private compile definition suggests no intended public API change, but does not by itself prove binary interchangeability. Keep it in a separate build/library directory; verify matching exported backend interface/version, actual library resolution and successful model execution. Reuse the preserved CUDA backend only through isolated process-local library paths, never by replacing original libraries or global links. Keep all other compilation options, CPU instruction targeting and model/runtime conditions matched as closely as possible.

For libgomp experiments on the original backend, start with process-local environment settings and preserve an unset/default control. Treat `GOMP_SPINCOUNT=1000` and `OMP_WAIT_POLICY=PASSIVE` as experimental scheduling choices, not quality-changing model settings. More sleeping can reduce CPU usage while increasing wake-up latency; more spinning can shorten waits while wasting cycles. The source sets `KMP_BLOCKTIME` if absent (`ggml-cpu.c:4509`), but that is not evidence that the GNU OpenMP runtime uses that Intel-runtime variable. No wait-policy or OpenMP-off improvement is claimed by this review.

## Final small-request review

The selected server uses `n_threads_batch` whenever `ubatch.n_tokens > 1` (`src/llama-context.cpp:1671,2752`). This includes small 12/30/4-token prompt pieces and 17-token speculative verification; only ordinary single-token decode uses `n_threads`. Therefore batch-thread tuning remains relevant even when most large prompt work is on the GPU.

Fresh prompt pieces consume successive tokens once (`tools/server/server-context.cpp:3974–4013`). The checkpoint saves the preceding recurrent state before the next piece executes (`:4050–4072`); no duplicated fresh-prompt evaluation was established. Per-request speculative settings are compiled out by `#if 0` (`tools/server/server-schema.cpp:197–227`), and live `cache_prompt` does not disable checkpoint creation. `--checkpoint-min-step` keeps near-end/last-user exceptions. Globally disabling context checkpoints can alter the 32-token CUDA offload routing and loses prefix-reuse snapshots; required speculative rollback state is separate. No checkpoint-removal patch was accepted from this review.
