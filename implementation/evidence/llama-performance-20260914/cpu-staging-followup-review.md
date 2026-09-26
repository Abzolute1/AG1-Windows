# CPU Q4/Q6 and multi-token staging follow-up

Read-only review of original `bd1bf025fc55ffa1fcb2ba6d8bb8805f35671d1f` during parent-controlled sequential inference. No compilation, hashing, CPU microbench or GPU work. The separately requested Q6 candidate was prepared only as new source artifacts under `/home/alex/Projects/llama-perf-20260914/cpu-q6-scales/`; no original or existing experimental source/build was edited. No performance claim is established here.

## 1. Narrow Q6 scale-vector reuse: prepared, low whole-model priority

`ggml/src/ggml-cpu/arch/x86/quants.c:2456` already computes all signed 16-bit scales for the Q8 sum correction. Lines 2488-2498 repeat sign extension for eight shuffled scale pairs. Reusing the expanded vector with two 128-bit-half duplicates and eight 256-bit lane-specific byte shuffles can replace eight 128-bit shuffles plus eight sign extensions. Six fewer vector operations at source level, subject to actual compiler assembly/register-pressure checks. Every integer product, sum, FP conversion, FMA and final reduction remains original.

Parent's selected-expert traffic estimate is only 6.6 MiB/token Q6 out of 293 MiB/token total, versus 115.5 MiB/token Q5. The Q6 patch is consequently a low-priority exact kernel experiment, not a plausible large all-round improvement by itself. Traffic share is not measured CPU time share. See the artifact README for an Amdahl bound and post-change gates. The exhaustive signed-scale-pair fixture is prepared but unrun.

Reject Q6 high-bit commuting as a source-count improvement: lines 2470-2473 already use one mask plus at most one shift for each two-bit field. Changing mask/shift order does not remove the duplicate shift present in original Q5. It may change register constants but offers no demonstrated benefit.

## 2. Unique gate/up quantized staging: real memory opportunity, no safe speed promise

`ggml/src/ggml-cuda/mmq.cu:190-234` already detects broadcast gate/up activations (`ne11 == 1`, selected experts > 1) and quantizes each token once. `quantize.cu:519-538` then writes that quantized block once per selected expert slot. Therefore proposing "quantize each broadcast token only once" repeats an existing optimization.

The allocation at `mmq.cu:205-207` still holds `B * selected_experts * padded_K` quantized values. `mmq.cuh:24-57` defines 144 bytes per 128 values. With K=2048, eight experts and B=1024, this is 18 MiB plus a small tile guard. A single-copy token buffer would be 2.25 MiB, saving **15.75 MiB of this temporary**; at B=2048 the saving would be **31.5 MiB**. These are allocation arithmetic, not measured reduction of the graph allocator's peak or process VRAM. Down activations differ per expert slot and cannot use this deduplication. Gate/up scratch lifetimes may reuse the same pool allocation, so do not add both savings as simultaneously live buffers.

It is not safe to shrink the allocation alone. Current MMQ input tile loads at `mmq.cuh:902-927` copy contiguous expert-sorted quantized rows into shared memory; the kernel uses a `ncols_y` stride and expert-sorted offsets at lines 1006-1052. A new opt-in tile-load variant would need to map each compact row back to the original token, e.g. from the true compact-row destination ID divided by selected-expert count. `mmid.cu:101` records that ID as `token * selected_experts + slot`.

Required scope and hazards:

- Gate only broadcast activations with CUDA Q4/Q5/Q6 MMQ, retaining original quantization layout and numerical dot code. Preserve non-broadcast, FP4 and non-MMQ paths.
- Preserve source activation strides while creating one quantized row per logical token; do not confuse original physical `sis1` with logical compact row numbering.
- Handle partial tiles without reading uninitialized ID entries or quantized rows. Zero/padding behavior must remain compatible with MMQ's unmasked tile loads.
- Handle all normal and Stream-K/fixup paths. In the partial Stream-K path, `ids_dst_shared[j]` can be deliberately reset to `j` (`mmq.cuh:1208`) for temporary output placement, so it cannot also serve as the true input-token mapping. Keep those roles separate.
- Validate duplicate IDs, uneven routing, output strides, graph replay and scratch lifetime. Instrument actual pool high-water and graph reservation before claiming a usable B/context increase.
- Existing contiguous tile reads would become indirect gathers of 144-byte row blocks. Lower writes/capacity may help, but added address generation, less coalescing and kernel specialization can hurt speed. A model benchmark is necessary; there is no defensible "without hurting speed" promise from source alone.

Recommendation: retain as a possible modest-memory CUDA experiment only if another 16-32 MiB actually crosses a measured allocation boundary. It is broader and riskier than the loader/CPU exact rewrites and is not implemented.

## 3. Q4 repack and expert-weight compaction: reject duplicate or weak proposals

- `ggml/src/ggml-cpu/repack.cpp:4600-4604` already selects the Q4_K 8x8 AVX2 format when row counts are divisible by eight. The AVX2 GEMV exists at `arch/x86/repack.cpp:1464` and GEMM at 2042. Merely "enable an AVX2 Q4 repack" is existing functionality; the parent's --no-host experiment changes both host pinning and buffer selection, so it does not isolate repack performance. Actual buffer logs are required before attributing the slowdown to either. Canonical CUDA_Host and CPU_REPACK have different compatibility with GPU staging; converting an arena dynamically is not a safe pointer-only change.
- Q5/Q6 repack selectors at `repack.cpp:4644-4667` are ARM-only. Enabling those formats on AVX2 without a matching kernel is incorrect. Writing a full Q6 repack kernel is disproportionately large for this placement's Q6 share.
- Canonical Q4 dot (`arch/x86/quants.c:2038-2123`) already uses AVX2 maddubs/madd, existing Q8 block sums for the minimum correction, and fixed-order FMA. AVX512/VNNI proposals do not apply to this Zen3 CPU. I found no comparably narrow, proven instruction-removing Q4 rewrite from this source read; arbitrary prefetching or changing FP association is not justified.
- The scheduler already groups and copies only used consecutive expert ranges (`ggml/src/ggml-backend.cpp:1813-1897`), including extra safe MMQ padding. It still reserves the original full tensor shape via `ggml_dup_tensor_layout` at 1560. Compacting selected experts would require remapping IDs, tensor dimensions/strides and graph reservation/lifetime. At large prefill batches the observed unique-expert count, not eight experts per token, determines the available saving; count that before investing. Do not claim an eightfold weight-buffer reduction from an eight-expert token when the batch collectively touches most experts.

No further source change is recommended solely on theoretical interest. Prioritize the already-built Q5 real-model ablation and loader memory experiment; profile residual time and actual peak memory before implementing a CUDA gather path or a new repack format.
