# Reproducing the investigation with another model

> Sanitized technical document. Personal paths, machine identifiers and application-specific records have been removed. Measurements are historical and hardware-dependent; they are not portable performance guarantees. Referenced raw evidence and source bundles have not been sanitized or included. This document alone is not a complete executable reproduction package.

## Three different meanings of reproduction

| Objective | What is preserved | What needs new work |
|---|---|---|
| Repeat the same Qwen experiment on the benchmark host | Exact tested binary bundle, frozen prompts, launch manifests, sampling, telemetry and comparison scripts | Idle GPU, new output prefix, matched machine/cache conditions and repeat measurements |
| Rebuild the implementation from source | Exact fork commit, complete six-file patch, before/after source hashes, original build configuration and incremental build plans | Separate checkout, compatible toolchain, complete rebuild and fresh correctness/performance validation |
| Apply the approach to another model | Source mechanisms, measurement method, correctness fixtures and lessons from rejected experiments | Architecture compatibility, new model baseline, tokenization, workload expectations, memory budget and parameter search |

The current binaries were linked incrementally using unchanged original objects and six replacement translation units. The source patch and build plans make those changes reviewable and reproducible in the preserved environment; they are not a fully portable, bit-for-bit clean-build lockfile. The tested bundle preserves exact binary bytes. Its host driver, standard libraries, hardware and existing model paths remain external dependencies.

## Evidence and implementation to retain

- Main measured report: before/after results, every meaningful experiment group, quality checks and caveats.
- Original launch and selected complete launch: actual argv/environment, not only a settings delta.
- Original build configuration, source identity, model metadata and tensor layout.
- Source/bundle README, six-file patch, source manifest, and binary manifest.
- Benchmark runner, paired comparison driver, API checks, frozen workloads, run catalog, and timing exclusions.
- Preserve the experiment directory `<EXPERIMENT_ROOT>` as well as this evidence directory. The report alone does not contain the binaries, all build plans or all test programs. Do not delete the preserved original object/build tree if replay of the incremental builds is required.

## What transfers, and what does not

| Mechanism | Reason it helped this model | Conditions to investigate for another model |
|---|---|---|
| Larger prompt micro-batches | More known prompt tokens amortize selected expert-weight transfers and GPU work | Measure actual prefill; workspace and KV requirements differ. Batch 4096 is a measured choice here, not a universal default. |
| Different CPU expert placement plus explicit GPU expert cache | Fits useful prompt batches while keeping recently used routed weights on the GPU during decode | Requires compatible routed-MoE execution and this fork's cache implementation. Dense models do not have these routed expert-cache opportunities. `--n-cpu-moe 32` refers to layers' expert storage, not 32 individual experts. |
| Cache eviction and execution settings | Avoids practically permanent protection of stale entries; selects the faster measured cached execution path | Revalidate across topic changes and routing distributions. Cache size 4480 MiB, reserve 512 MiB and hot-use threshold are specific experimental choices. Setting a huge hot-use threshold approximates LRU over realistic runs; it is not a new cache algorithm implementation. |
| Read-only file-backed CPU expert weights | Avoids a large duplicate permanently pinned host allocation | Check memory pressure, major faults, cold first-request latency and supported tensor layouts. Lower readiness time can defer I/O into the first answer. |
| Two-slot pinned transfer staging | Transfers selected mapped expert ranges through bounded pinned buffers with event-fenced reuse | The selected callback explicitly supports eligible CPU_Mapped Q4_K/Q5_K/Q6_K tensors. Other quantizations/layouts can fall back and must not inherit the speed claim. Two 2 MiB slots were measured here. |
| Wider ordinary cache coverage | Keeps 17-token speculative verification and small prompt tails on the useful cache path | Original target routes eight experts per token. Selected ordinary capacity is 32 tokens / 256 rows; other expert counts and graph layouts require routing/correctness checks. Fused operations retain their original limits. |
| ngram-mod speculation | Reuses candidate token sequences from available text/history, then verifies them with the target model | Likely benefit depends on repetition and acceptance. Exact editing improved greatly; ordinary prose did not gain merely by enabling n-grams. Test fresh answers separately from learned repeated answers. |
| Six decode / twelve batch threads | Balances remaining host work against synchronization and scheduling costs | CPU topology and backend behavior differ. Multi-token verification also uses batch threads in this server path. Original CPU-range/poll flags were ineffective here; do not assume every server revision behaves that way. |

These are hypotheses for a new model until measured. The patch is based on TurboQuant-fork commit `bd1bf025fc55ffa1fcb2ba6d8bb8805f35671d1f`, not a patch guaranteed to apply to current official upstream. Rebase/port it in a separate tree and inspect existing upstream functionality before adding duplicate machinery.

## Procedure for a new model

1. Create independent source, build and result directories. Preserve the original binaries and weights; use a separate loopback port.
2. Record the model architecture, quantization, experts per token, context support, tokenizer, KV precision and actual tensor placement. Measure its own baseline.
3. Adapt the benchmark harness paths, model identity, baseline arguments and result directories. Historical harnesses may contain hard-coded defaults and are not generic model launchers.
4. Freeze representative prompts and exact expected answers. Render the new model's chat template and regenerate token IDs; do not reuse another tokenizer's IDs.
5. Measure fresh short requests, long ingestion, source review, exact editing, structured extraction, tool calls, first-response latency and near-capacity context. Hold sampling and token caps constant.
6. Sweep runtime configuration before source changes: batch and micro-batch size, CPU expert placement, cache budget and thread counts. Compare prefill and decode separately and verify memory headroom at full context.
7. Isolate source changes. Validate loader lifetime, staged-transfer byte equality, event ordering, cache routing and supported tensor layouts. Unsupported paths must fall back correctly.
8. Test speculation with fresh processes and previously unseen answers. Separately measure repeated-answer behavior and draft acceptance; acceptance alone does not establish speed.
9. Confirm candidates in balanced original/candidate order with repeated fresh processes. Use at least five matched pairs for the main workloads. Exclude compiler, GPU-test and heavy-I/O interference.
10. Check exact outputs, held-out quality, full context, prefix reuse, interruption and recovery. Numerical changes require comparisons against repeated unmodified controls as well as synthetic fixtures.
11. Separate warm-cache loading, cold-file loading, server readiness and load-plus-first-response. Lazy loading can move costs into the first answer. Never label a warm readiness measurement as cold startup.
12. Package the exact selected source patch, build commands, compatible libraries and sanitized launch manifest. Recheck original file hashes. Treat integration into an application as a separate reversible task.

## Acceptance record for each model

Record original → candidate medians/ranges for prefill, decode, streamed TTFT, full response, warm/cold readiness, startup + first response, VRAM, mapped/anonymous/shared memory, available RAM, CPU/GPU utilization and power/temperature. Include repetition counts, cache/history state, context, batch/ubatch, offload, cache budget, precision and sampling. Record exact correctness scores, probability-level differences, stability and unsupported cases. Declare the best measured tradeoff; do not describe an unmeasured transfer as a universal acceleration.

The reusable outcome is the implementation plus this measurement procedure. The exact 4096/4096, CPU-layer placement, cache budget, thread counts and speed multipliers must be earned again for a different model.
