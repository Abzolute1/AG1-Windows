# Kernel opportunities and applicability

## Exact-mask warp skip (prepared, not yet tested)

The Q8_0 vector Flash Attention path for head256 still computes K dot products and reads/dequantizes V for rows whose mask is negative infinity. Each warp handles32 KV rows independently within the loop; there is no block-wide barrier until the loop ends. For a wholly masked warp iteration, the normal recurrence adds zero and scales the accumulated result by one. A warp-uniform early continue can therefore avoid that work without dropping any unmasked attention entry. The isolated candidate restricts this to one query, Q8_0 K/V, head256, zero bias and cache length at least4096. It preserves the accumulation order for retained tiles.

Twenty synthetic GPU cases cover contexts2048/4096/21504/83200 and block-sparse, scattered, dense, empty and tail-only masks. Two balanced process pairs must produce identical output bytes before real-model runs. Runtime measurements and long-context retrieval are still required; no gain is claimed yet.

Upstream sparse-FA PR27970 is merged, but its enabled shapes are512/512 and576/512 on the MMA F16 path. Flash-Next here has256/256 heads and quantized Q8 cache dispatches single-query decode through the vector path. Merely enabling the commented graph hint does not enroll this case. The QSA graph's expanded top-k output already contains token positions, bounded by indexer_top_k+compress_ratio-1 (2051 here); it is not a2048-block/8192-token budget.

Primary source: https://github.com/ggml-org/llama.cpp/pull/27970 and the downloaded b10909 source's fattn.cu, fattn-vec.cuh, fattn-mma-f16.cuh and models/qwen4exp.cpp.

## Force-MMQ flag is not the missing IQ1_M optimization

In this revision, ggml_cuda_should_use_mmq first rejects unsupported types. IQ1_M is not in its supported switch, whereas MXFP4 and IQ2_S are. Supported types return true on Turing-capable tensor-core hardware, which includes this Ada GPU, before GGML_CUDA_FORCE_MMQ is considered. Therefore force-MMQ does not enable an IQ1_M MMQ kernel or alter the already-selected path for supported types on this machine. No benchmark of that no-op flag is needed.

An actual IQ1_M fused matrix kernel would be a larger implementation: IQ1_M has subblock scale and delta structure that is not interchangeable with IQ1_S. Reinterpreting or re-quantizing those weights is not an acceptable lossless optimization. Profiling showed many IQ1_M dequantization launches, so a correctly implemented fused or grouped path remains a technically plausible opportunity. It has not been implemented or benchmarked here and must not be described as an achieved improvement.


## Large-prefill transfer scheduling (prepared, not built)

`prepare-bulkcopy.py` creates an isolated ggml-backend.cpp overlay on newer Unsloth329b6160f. For explicitly enabled large MoE batches it copies the complete unchanged expert tensor to its existing GPU buffer, avoiding the routing-ID readback/synchronization used to choose sparse transfer ranges. The unchanged GPU op still uses the original IDs and quantized weights. At1024tokens and10experts/token there are10240 selections among512experts, so much of the tensor may already be needed; actual skew can make a full transfer waste bandwidth, which must be measured. Small batches retain the original sparse path. This may combine with private pinning or a larger bounded staging ring to reduce serial CPU/GPU waits without34GiB of additional permanently pinned copies. It is not yet a measured improvement.


## Speculative recurrent-state copy fan-out (source review only)

`qwen4exp.cpp::build_conv_state_at` emits one tail-copy subgraph per rollback slot, for both convolution histories. With large draft depth and only one actual token, many slots copy the same earlier tail into different destinations. Omitting slots is not automatically safe: the recurrent memory API permits bounded rewind and reads indexed snapshot planes. A potentially lossless optimization is to gather all required tail columns with one index tensor and copy them to the existing strided snapshot planes in one operation, preserving every slot exactly. This would reduce small-kernel launches without reducing rollback capability. It should only be implemented if speculative runs demonstrate a meaningful overhead and an independent GPU fixture establishes identical snapshot bytes, including multisequence and short-batch cases. No such patch has been implemented or benchmarked yet.


## N-gram checkpoints versus existing recurrent snapshots (prepared)

The current `common_params_speculative::need_n_rs_seq()` enables fast recurrent rollback planes for MTP and other learned drafts, but returns0 for ngram-mod. Server code therefore calls `ckpt.update_tgt(...PARTIAL_ONLY)` before each nonempty n-gram proposal on a hybrid target and restores/replays after partial acceptance. The first original ngram4 edit accepted713/716 draft tokens and improved generation to39.25tok/s; ngram8 accepted792/800 but only reached37.94tok/s. These are initial screens, not final accepted gains.

`prepare-ngram-rs.py` is an unbuilt opt-in common.cpp overlay that gives ngram-mod the existing recurrent snapshot depth matching its draft maximum. It preserves the existing acceptance and exact rollback code. It may avoid repeated host checkpoint transfers but increases recurrent state memory and per-step snapshot work; small/free-form requests can therefore regress. `further-source-screen.py` tests both modes at identical CPU44 placement and90000context, depths4/8 in reverse order, including edit/prose/repetitive workloads and prefix-reuse diagnostics. The proposed convolution-copy fusion above becomes relevant to this snapshot-enabled path or MTP, not the current stock ngram-mod path. No snapshot optimization has been benchmarked yet.

### CUPTI time-share check before considering a custom IQ1_M kernel

The instrumented original review prefill recorded 85,268 IQ1_M dequantization launches, but their summed GPU duration was only 0.506 s. All GPU kernels summed to 5.128 s, while H2D copies alone summed to 10.102 s and moved 173,038,170,220 bytes (161.16 GiB). The host cudaMemcpyAsync API summed to 13.338 s. These are instrumented, possibly overlapping activity durations, not additive wall-time components or uninstrumented speed estimates.

A fused IQ1_M kernel could remove launches and temporary dequantization, but dequantization time alone does not justify a large custom kernel project before transfer/synchronization experiments. The existing fallback uses F16 dequantization; replacing it with a Q8 activation MMQ path would also require numerical assessment rather than being automatically classified as identical arithmetic. Prioritize the prepared whole-expert prefill copy and pinned-transfer tests, then revisit kernel work only if profiling a surviving candidate shows a material remaining bottleneck. Raw evidence: `cupti-original-summary.json`.

### AG1 readiness includes material verification

Source inspection of `product/crates/ag1-platform/src/llama/native.rs` shows an in-memory `MaterialVerifier` cache keyed by canonical path plus device/inode/length/mtime/ctime and expected digest. A cache miss streams SHA-256 through every required material; `materials.rs::runtime_materials` includes all model shards. Thus a fresh native host may read the full79GiB model before llama.cpp starts, while later loads in that host can use the verified metadata cache. This is not measured app timing yet. Final AG1 validation must separate material-verification time, runtime load, and warm-model task latency. A3-second warm native llama-server load must not be presented as3-second first AG1 app readiness. Do not weaken integrity verification merely to improve this number.
