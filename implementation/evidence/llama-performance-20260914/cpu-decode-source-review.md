# CPU expert decode: bounded source review

Reviewed `/home/alex/Projects/llama-perf-20260914/original-experimental` at `bd1bf025fc55ffa1fcb2ba6d8bb8805f35671d1f`. No GPU work, builds, model changes or source implementation were performed. References are relative to this tree. Model layout comes from `model-layout.json` in this evidence directory. Suggested improvements below are hypotheses, not benchmark results.

## Established execution constraints

Canonical `MUL_MAT_ID` uses `ggml_compute_forward_mul_mat_id_impl` (`ggml/src/ggml-cpu/ggml-cpu.c:1585`). It quantizes F32 activations to the weight type's dot-product partner format, groups token/selected-expert rows, initializes per-expert work counters, and then calls the existing dot kernels. The single-row inner path calls `vec_dot` once per output row (`:1514`, `:1568`). Q4_K and Q5_K have AVX2 implementations in `ggml/src/ggml-cpu/arch/x86/quants.c:2038` and `:2216`; their `nrc` assertion is one. Both unpack scales and reload the shared Q8_K activation block for each output row.

For single-token decode, this general-purpose routine still initializes counters for all 256 experts (`ggml-cpu.c:1778`) and every worker scans all expert entries (`:1786`), although only eight are selected. When each expert has one token row, the chunking logic can fall back to static per-thread output-row slices (`:1800` onward), so removing atomics is not equivalent to removing all scheduling cost: the decode path may already avoid atomic fetch-add for the actual work. Measure the metadata initialization/scanning separately.

Separate gate and up graph operations share `cur` and `selected_experts` (`src/llama-graph.cpp:2138`, `:2151`). Each invocation nevertheless requantizes its input (`ggml-cpu.c:1659`) and rebuilds row metadata. Existing CPU fusion is tied to the MoE-cache provider (`:3244`), requires matching input/ID pointers and shapes (`:3334`), and returns to ordinary execution when no cache node is available (`:3480`). Even an active cache fusion calls the general implementation separately for CPU misses in up and gate (`:3491`, `:3494`). It is not a general CPU paired-matvec optimization.

The existing Q4_K repack selection supports AVX2 (`ggml/src/ggml-cpu/repack.cpp:4600`), while the Q5_K/Q6_K choices shown at `:4643` onward require ARM NEON features. That establishes a gap in available Zen3 layout variants, not evidence that implementing a Q5_K repack would beat its canonical AVX2 kernel.

The original build uses OpenMP. Its graph barriers call `#pragma omp barrier` (`ggml-cpu.c:631`); the custom `--poll` loop is excluded. Interpret CPU utilization alongside useful graph time and libgomp wait-policy measurements.

## Upper bounds and traffic scale

Using the first 20 expert layers, eight selected experts, each tensor's actual quantization and its declared dimensions, the nominal selected-expert weight payload is **293.0625 MiB per generated token**: Q4_K 171 MiB, Q5_K 115.5 MiB and Q6_K 6.5625 MiB. This excludes nonexpert tensors, activations, allocation padding and additional reads; it is not a measured DRAM traffic counter. Cache reuse can reduce DRAM reads, while poor access patterns can increase traffic.

At the theoretical dual-channel DDR4-3600 rate of 57.6 GB/s, that payload corresponds to approximately 5.34 ms if every selected weight byte comes from DRAM once. Actual sustainable bandwidth is lower, but cache hits mean this calculation is not a universal latency lower bound. Gate/up input-quantization sharing saves small activation work; it does not eliminate the 293 MiB weight payload.

The parent supplied a profile with roughly 49.8% CPU graph time and 43.3% CUDA waiting during medium-prompt decode. If these fractions remain unchanged otherwise, making the entire CPU graph twice as fast yields only `1/(0.502+0.498/2) = 1.33x` total speed. Eliminating CPU graph time entirely gives a theoretical 1.99x ceiling. The individual experiments below address only subsets of CPU time, so their realistic upper bounds are much smaller. Fractions can change with placement, batching and wait policy; reprofile the final candidate rather than applying this bound blindly.

## Ranked source experiments

### 1. Narrow single-token canonical MUL_MAT_ID fast path

At `ggml-cpu.c:1585`, add an isolated opt-in path only for one token, canonical eligible tensors and the proven no-cache/no-row-mask case. Enumerate selected ID slots directly and assign disjoint `(selected-slot, output-row-block)` work to workers. Reuse existing activation quantization and Q4_K/Q5_K/Q6_K dot kernels. This avoids a 256-entry expert grouping table and counter initialization/scanning for an eight-expert decode. Keep the general path for prefill and all unsupported cases.

Correctness requirements: preserve activation block quantization exactly; handle gate/up broadcast input (`ne11=1`) and per-expert down input (`ne11=8`) correctly; retain strides, bounds and output-slot ordering; preserve repeated IDs as distinct output slots; preserve the initial activation-ready barrier and graph completion barrier. Do not remove a barrier based solely on the number of active experts. Initially reject cache-active and masked executions rather than accidentally bypassing GPU cache hits. Use canonical existing dot-product operation order so this scheduling-only experiment can seek exact output agreement.

Bound: only metadata/scheduling savings, not weight traffic or arithmetic. The 256-counter storage is approximately 16 KiB, and only eight matrices are nonempty. A substantial whole-response win is unlikely from this change alone; its value is a small low-math-risk candidate and a clearer decode specialization for later work.

### 2. Paired gate/up execution sharing input preparation

Build a CPU-only paired operation/fusion using the existing matching checks near `ggml-cpu.c:3334`, but decouple eligibility from successful GPU-cache allocation. First confirm in scheduler traces that both matvecs are in one CPU graph; GLU may be assigned to another backend, in which case fuse only the pair and leave GLU and cross-backend semantics intact. Quantize the shared activation once, build the selected-ID mapping once and compute both outputs using their unchanged canonical dot kernels. Preserve both intermediate outputs unless graph-use counts prove they can be eliminated. Do not implement a process-global pointer cache: addresses are reused and activation contents change each token.

Correctness requirements: exact shared activation and ID tensors, compatible partner quantization, dimensions, strides and output disjointness; no intervening observable operation; graph lifetime/use-count and backend placement proof. Workspace must explicitly own the shared quantized activation until both outputs finish. Existing MoE-cache fusion has an independent asynchronous hit/miss fallback contract, so initially restrict the new path to cache-disabled execution or integrate that contract explicitly. Preserve F32 rounding between matvec and activation stages; using a fused FMA or omitting stored rounding can change outputs.

Bound: removes one input conversion, repeated metadata work and potentially one internal preparation barrier per pair. It still reads both weight matrices and performs both dot products. At dimension2048 the common activation has only eight Q8_K blocks, so activation-conversion savings alone cannot explain a large decode gain. This experiment is justified only if measured CPU setup/barrier costs are material.

### 3. Canonical Q5_K two-output-row AVX2 microkernel

The down projection's input dimension is512, only two K-quant blocks, while its output is2048 rows. A specialized paired-row routine adjacent to `ggml/src/ggml-cpu/arch/x86/quants.c:2216` could load each Q8_K activation block and its block sums once, process two independent canonical Q5_K weight rows, and maintain separate accumulators. Dispatch from the one-chunk loop (`ggml-cpu.c:1567`) only for supported alignment/dimensions, retaining the single-row tail. This needs no weight repacking or model copies and targets repeated activation loads, short-loop setup and instruction overlap in a quantization type without Zen3 repack support.

Correctness requirements: preserve each row's scale/high-bit extraction, integer saturation bounds, F32 accumulation/reduction order and tail handling. Keep the existing kernel as reference/fallback. Two-row interleaving increases live vector state on AVX2's limited register file and may cause spills; inspect generated assembly before interpreting a benchmark. A speculative prefetch by itself is a lower-value variant because selected expert rows are sequential once identified and hardware prefetch may already suffice. Never issue out-of-allocation prefetch pointers without valid bounds.

Bound: Q5_K supplies approximately39.4% of the nominal CPU expert payload here, but traffic share is not time share. Even if it were that fraction of CPU graph time and this kernel magically doubled all Q5_K work, the total-speed bound would be only about1.11x. A paired-row routine saves a much smaller portion and cannot halve weight bandwidth. A real benefit needs meaningful vector-kernel and end-to-end results.

## Required gates before keeping a source variant

The applicable original `AGENTS.md` explicitly requires `test-turbo-quant`, `test-quantize-fns`, a full CPU+CUDA `test-backend-ops` sweep and `llama-bench` coverage before touching quant/backend code. These three proposals modify backend or quant execution, so plan those gates rather than claiming a smoke test satisfies the repository rule. Check actual executed cases: the same instructions warn that zero executed cases can still report success.

In addition, require targeted canonical `MUL_MAT_ID` cases with one token, eight selected experts, 256 total experts, Q4_K/Q5_K/Q6_K, both2048-to512 and512-to2048 shapes, multiple thread counts, repeated/edge expert IDs, broadcast/per-expert activations and nontrivial strides. Gate/up fusion also needs graph-use and interleaving negatives, and cache-enabled fallback coverage. Run repeated fixed-input Qwen3.6 inference plus extraction/correctness tasks, prompt processing and long-context regression checks. A microbenchmark improvement alone is insufficient.

No source variant should be retained merely because the full investigation seeks a substantial all-round win. Reject losses and changes within measurement noise.

## Is phase-dependent repacking practical without duplicating all weights?

Not as a simple flag or in-place reinterpretation. The existing repack buffer sets tensor-specific traits, rewrites layout on `set_tensor`, requires a whole-tensor load, and disables `get_tensor`/`cpy_tensor` (`ggml/src/ggml-cpu/repack.cpp:4726` through `:4762`). Packed bytes are not canonical GGUF bytes even when tensor type and nominal size remain unchanged. The current allocator/backend contract intentionally prevents ordinary GPU copying of that representation.

A permanent CPU-only placement can store one repacked copy and discard its transient canonical source pages after loading. That avoids two resident copies but also gives up direct canonical expert upload for GPU prefill/cache. The original model file must remain read-only and unchanged. Toggling between layouts every phase would require reconstructing one from the file or an explicit inverse transform and paying large host read/write costs, plus synchronization and graph/backend-cache invalidation. A single-token decode step must never perform whole-model repacking.

A bounded hot-expert repack cache can limit duplication to selected experts, but adds admission, eviction, invalidation, per-expert dispatch, page accounting and canonical-versus-packed routing. Repeatedly packing a missed expert can cost more memory traffic than its matvec saves. Memory-mapping canonical data does not make its pages free in RSS or page cache, especially when they are CUDA-registered. This is a larger subsystem, not one of the three recommended initial source experiments.

For this investigation, first measure permanent `--no-host`/existing Q4_K repack and quant-aware placement. If prefill and decode prefer different execution paths, prefer retaining canonical weights plus a small proven CPU kernel improvement before considering phase-dependent whole-weight transformations.
