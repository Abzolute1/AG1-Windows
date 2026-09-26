# Qwen3.8 Flash-Next performance investigation

> Sanitized technical document. Personal paths, machine identifiers and application-specific records have been removed. Measurements are historical and hardware-dependent; they are not portable performance guarantees. Referenced raw evidence and source bundles have not been sanitized or included. This document alone is not a complete executable reproduction package.

**Recommendation: retain the original default and keep these as optional acceleration configurations.** Most of the native editing gain comes from stock ngram drafting on the original runtime (1.94×), without the source candidate’s large private-memory cost. The combined candidate is useful for a model kept resident through long inputs and editing, but is not an across-the-board upgrade: it commits about 34 GiB of private anonymous host memory, slows repetitive rewriting, and took longer to load.

## Model and baseline

Model: Qwen3.8-Flash-Next-AD-3.84bpw-IQ4_XS-M64, with unchanged model weights.

- Original runtime: Unsloth b10715-mix-86bd2d3, build 10715, source `92cedc8679d145902ead3f006258e8672eac11e6`. Runtime reports 0.3.0-dev. compatible CPU and CUDA toolchains, multi-architecture CUDA build including the target GPU architecture; selected CPU backend haswell.
- Original executable: `<LOCAL_PATH>`.
- Requested context 90,000 (allocated 90,112), Flash Attention on, Q8_0 K/V, one slot, batch/microbatch 1,024/1,024, 12 decode/24 batch threads with strict 0–11/0–23 affinity, poll 100, CPU MoE 42, automatic GPU layer offload, fit off. No speculative drafting was configured originally.

```text
<SERVER_EXECUTABLE> --model <MODEL_FILE> --alias Qwen3.8-Flash-Next-AD-3.84bpw-IQ4_XS-M64 --host <LOOPBACK_ADDRESS> --port <BENCHMARK_PORT> --flash-attn on --ctx-size 90000 --cache-type-k q8_0 --cache-type-v q8_0 --no-webui --jinja --parallel 1 --fit-target 1024 --batch-size 1024 --ubatch-size 1024 --threads 12 --threads-batch 24 --cpu-range 0-11 --cpu-strict 1 --cpu-range-batch 0-23 --cpu-strict-batch 1 --poll 100 --slot-prompt-similarity 0 --no-mmproj --fit off --n-gpu-layers auto --kv-unified --n-cpu-moe 42 --metrics --verbosity 2
```

Configure CUDA and shared-library search paths for the selected build. No experimental GGML/OMP overrides were inherited by the baseline. See the captured environment for exact values.

## What “n-gram” means in this model

Flash-Next’s learned PLE subsystem performs bigram/trigram-dependent table lookup as part of the target model computation. Its Q5_1 table occupies 35.763 GiB and emits 16 rows/token. This is separate from the runtime’s optional ngram-mod speculative drafting. PLE has no draft acceptance rate. Runtime speculation proposes tokens, then uses the unchanged target model to verify them. An additional existing Q8 MTP draft head was evaluated separately.

The GGUF filename is not an accurate inventory of tensor quantization. Actual tensors include 48 MXFP4 down-expert tensors, 72 IQ1_M and 24 IQ2_S gate/up-expert tensors, 666 Q8_0 tensors, 388 F32 tensors, 24 BF16 tensors and one F16 tensor, plus the Q5_1 PLE table. No IQ4_XS tensor is present. Quantization types were read directly from the GGUF headers. Cache eligibility for existing IQ1_M experts changes placement only; it does not re-quantize weights.

## Baseline measurements

Three repetitions per workload, 24 native inference requests in baseline-01. Fresh prompt cache is disabled except the explicitly warmed conversation fixture. Fixed greedy sampling: temperature 0, top_k 1, top_p 1, min_p 0, repeat_penalty 1, seed 777. Frozen prompts and token IDs live in `prompts.json`. Prompt-cache warmth and OS file-cache warmth are distinct. The first review had slower page-cache behavior; the table reports all-three medians, not its best run.

| Workload | Input tokens | Prefill tok/s | Decode tok/s | TTFT s | Response s |
|---|---:|---:|---:|---:|---:|
| A: fresh long ingestion | 21308 | 305.46 | 25.97 | 69.762 | 77.978 |
| B: code review | 6250 | 302.03 | 27.99 | 20.696 | 29.781 |
| C: exact source edit | 1006 | 296.56 | 29.02 | 3.393 | 35.679 |
| D: ordinary prose | 74 | 61.12 | 29.19 | 1.211 | 9.946 |
| E: tool JSON fixture | 70 | 58.28 | 28.21 | 1.202 | 2.974 |
| F: cached conversation | 4243 | 66.56 | 28.36 | 0.978 | 9.991 |
| G: repeated code rewrite | 500 | 193.23 | 29.09 | 2.588 | 17.577 |
| H: free-form reasoning | 110 | 76.29 | 29.07 | 1.443 | 10.189 |

Conversation prefill speed measures only the newly evaluated suffix, not all 4,243 tokens. The short tool JSON fixture is supplemented by native function-call validation. Native edit outputs commonly omit the final newline; content agreement is not strict file-byte correctness. Short 256-token screens do not satisfy full 500-word review/prose requests; those require the final quality pass.

Initial observed model load: 85.443s; this was not a cold-boot/drop-caches test. Later original warm loads were around 3.4–5.4 s. Baseline peak total GPU memory: 14753MiB, including desktop use. Process RSS is predominantly reclaimable mmap pages; it must not be equated with private committed RAM. Raw CPU/GPU utilization, clocks, power, RAM, faults and I/O are preserved per request.

## Measurement interpretation

- Compare native prefill and common decode rates separately. Original native decode counts predicted_n−1; the older fork counts predicted_n. `RUNS.md` normalizes both to `(predicted_n−1)/predicted_ms`. Raw timings remain unchanged.
- Resource samples occur about every 0.5 s. CPU utilization 100% means one logical CPU. Early VRAM readings include desktop use; later runs also record process-specific NVML memory.
- Instrumented CUPTI/perf results locate bottlenecks and are excluded from speed comparisons. The stale-marker build-overlap arm and interrupted CPU-flat arm are explicitly excluded in `invalid-runs.json`.
- Same-file experiments keep model, quantization, context, sampling and other non-variable settings fixed. The optional higher-precision alternative is a separate model comparison.

## Measured bottlenecks

Original review prefill moved roughly 161 GiB of host-to-device data for 6,250 input tokens. CUPTI summed H2D copies to 10.10 s and the host cudaMemcpyAsync API to 13.34 s. These overlapping instrumented activities are not additive wall-time components. IQ1_M dequantization had 85,268 launches but only 0.506 s summed GPU duration. Transfer/synchronization work therefore deserves priority over a kernel rewrite based on launch count alone.

Original decode heavily exercised CPU expert dot products and OpenMP waiting while GPU utilization was around 42%. The measured CPU sample shares included MXFP4/IQ1_M/IQ2_S arithmetic and substantial waiting. Prefill and decode therefore need different solutions; making larger GPU batches fit by putting more experts in RAM can help one and harm the other.

## Experiments and current status

`RUNS.md` is the complete generated run catalog, with separate workload medians and ranges, failures, exclusions and configuration links. `STATUS.md` records checkpoints. `kernel-opportunities.md` and `turbo-source-review.md` explain the source hypotheses. `REPRODUCTION.md` describes the protocol. All source overlays remain in separately named experiment directories.

### Experiment families and rejected alternatives

The complete run catalog retains individual configurations, raw results, failures and exclusions. These historical screens explain which paths were carried forward; their values are not substitutes for the final balanced comparison.

| Family | Finding and consequence |
|---|---|
| CPU threads, affinities, polling and OpenMP | Fewer decode threads, passive waiting and poll=0 generally lost speed. Twelve verification threads were useful for longer ngram drafts; six verification threads across one chiplet were slower. Lower CPU utilization alone was not considered a win. |
| Batch size and expert placement | Original batch 2048/CPU 44 reached about 411 review-prefill tok/s, with a small decode loss; CPU 42 at 2048 exhausted VRAM. Larger staged batches reached about 469 prefill but only about 21 decode. The optional original 1536 follow-up was deferred; the final candidate retains batch 1024 and CPU 42 so it does not claim a batch-size gain. |
| Newer builds and CUDA graphs | Fresh Unsloth/upstream GPU-specific builds did not automatically beat the original. Disabling CUDA graphs reduced decode to about 23–25 tok/s. Keep graphs enabled. |
| CPU arithmetic/scheduling kernels | Paired MXFP4 rows improved a microbenchmark by about 6% without a useful application gain. Flat CPU scheduling produced only a small newer-build screen gain; the Turbo variant was slower in real inference despite exact synthetic output comparisons. Microbenchmarks alone did not justify either change. |
| Host registration and staging | Read-only registration failed with driver error 801. Private writable mappings preserve file contents and support registration. They improved prefill but increase memory commitment and startup cost. Bounded staging avoids pinning all experts but has shown workload-dependent results. |
| Fixed expert pools | Many pool sizes slowed decode to roughly 18–26 tok/s. An efficient 80-slot pool reached about 30 on prose without a general win. These bundles expand context-parameter ABI and cannot be mixed with ordinary libllama/common libraries. |
| Turbo dynamic cache | Cache budget, eligible expert size, placement, overlap, asynchronous fills, churn and fusion settings trade prefill, prose, edit speed and VRAM. Lowering the eligibility threshold moves existing IQ1_M bytes; it does not change quantization. Inspect actual allocated capacity rather than requested capacity. |
| Runtime ngram drafting | Longer drafts substantially help source editing/copying; repetitive rewrites can regress. Acceptance is not itself a speed metric. Recurrent snapshots and adaptive backoff are separately evaluated rather than assumed to improve the same workloads. |
| Existing Q8 MTP head | At matched CPU 46, depth 2/depth 4 improved edit decode from about 27.15 to 32.91/37.09 tok/s, while prose fell from 27.20 to 26.71/22.15 and reasoning also slowed. It is less promising than ngram drafting for the measured editing task. |
| Masked attention | The synthetic sparse case improved 3.443× with 20 exact fixture outputs. Real long-context decode improved about13%; fresh response latency improved much less. Real affected-path numerical checks and task quality are reported separately. |
| Whole-expert transfers | Avoiding selected-ID readback slowed unpinned review prefill in both process orders. With private pinned mappings it was approximately tied with selected-expert transfers, not a substantial additive gain. |

Turbo's current cache path uses 64-bit logical-row masks. Ten experts per token means long speculative verification batches exceed that path and use the stock bypass. Raising the limit beyond 64 without redesigning the masks would be incorrect. This helps explain why more cache hits or longer drafts do not automatically translate into better verification throughput.

### Results since the authorized reboot resume

| Experiment | Matched control | Experimental result | Interpretation |
|---|---|---|---|
| 84,859-token retrieval, masked attention | Newer base decode 20.59/20.65 tok/s | 23.27/23.37 tok/s | About 13% faster decode; only about 0.9% lower fresh response time because prefill dominates |
| Source edit, fixed ngram16 with 12 verification threads | No drafting 26.72/28.94 tok/s | 51.04/48.32 tok/s | Substantial repeated improvement on copying most of a source file |
| Source copying, same drafting setup | 26.96/25.65 tok/s | 59.44/61.42 tok/s | More than twice the measured decode rate in both passes |
| Repetitive rewrite, same drafting setup | 26.99/26.47 tok/s | 23.56/19.17 tok/s | Draft verification is unprofitable on this workload |
| Repetitive copying, same drafting setup | 27.09/28.65 tok/s | 34.20/36.65 tok/s | Some partially accepted drafts still save time; acceptance alone is an inadequate policy signal |

All six long-context retrieval runs (original, newer base, masked candidate, each twice) passed strict JSON and exact factual checks. The candidate's mean TTFT was 297.91 seconds versus 299.62 for the newer base and 306.42 for the original. These are fresh-context requests, not cached chat latency. See long-context results and quality checks.

Across the six no-draft, fixed-draft and adaptive-draft process runs, each of the five workloads produced identical output bytes in every arm. Source edits still omitted the requested final newline, and repetitive rewrites still used an unwanted outer fence in every arm. These shared defects remain failures of strict formatting; content agreement does not erase them. See drafting quality checks.

The first adaptive policy (pause for 1,024 tokens after fewer than 85% of 32 proposed tokens are accepted) recovered only part of the repetitive-rewrite loss. It also stopped too early on profitable repetitive copying. It is not ready as a general automatic default.

Parallel private-page preparation did not help: median load was 22.631 seconds with six workers and 21.109 seconds with twelve, compared with 20.627 seconds with none. Subsequent combinations therefore use no extra workers. See all nine load samples.

The warm-file figure must not be generalized to ordinary restarts. In the subsequent pinned-plus-bulk comparison, `cow-bulkcopy-bulk-0` took 67.060 seconds to load without explicit prewarming after another pinned process exited. Its corresponding first control had taken 23.755 seconds from a different inherited file-cache state. Pinning can leave fewer source pages cached for the next process. These differing load samples are retained; the bulk-transfer flag acts during inference and should not be credited or blamed for the cache-state load difference.

Numerical checks require care: prose/edit fixed-prefix comparisons below the masked-attention activation threshold were identical, but they did not exercise the patch. A separate 6,250-token review diagnostic did exercise it. Candidate versus control agreed on 62/64 greedy positions; a second unmodified control agreed with the first on 63/64. Reported top-32-plus-residual distributions also varied within the control. This does not establish bit-exact real-model equivalence or isolate every change to the patch. The earlier exact synthetic kernel fixtures and successful real retrieval checks are separate evidence. See affected-path comparison and control repeat.

### Candidate selected for focused validation

`R/closeout-candidate.json` uses `bundle-closeout-cow-mask`: the screened private-mapping loader and masked-attention CUDA library, with stock ngram16 drafting and twelve verification threads. The primary model, 90k context, Q8 K/V, batch 1024 and CPU 42 placement remain unchanged. The assembly checks matching public ABI headers and records every library hash in the assembly manifest.

This combination targets the three demonstrated opportunities: prompt processing, long-context decoding, and source copying/editing. Its combined behavior is measured in the final comparison below. Adaptive backoff, recurrent snapshot extensions, whole-expert copying, staging and the Turbo runtime are not part of this candidate. The comparison includes both the untouched original and the same stock drafting configuration on the original runtime, so the source changes must earn their additional complexity and startup cost.

## Final full-answer comparison

Two reversed process orders, one request per workload in each process. Values are means of the two observations; the machine-readable summary retains samples and ranges. Decode uses the common token-count convention. These are bounded measurements, not confidence intervals or a universal speedup.

| Workload | Original decode tok/s | Drafting only | Combined candidate | Original response s | Drafting only s | Candidate s |
|---|---:|---:|---:|---:|---:|---:|
| review | 26.76 | 26.88 | 26.96 | 75.14 | 73.36 | 63.70 |
| edit | 27.25 | 52.87 | 56.12 | 38.15 | 21.14 | 19.11 |
| prose | 27.92 | 28.30 | 29.26 | 25.94 | 25.54 | 26.23 |
| tool | 26.40 | 27.38 | 28.17 | 3.19 | 3.00 | 2.53 |
| conversation | 27.50 | 27.29 | 26.53 | 66.53 | 67.09 | 68.97 |
| repetitive | 26.76 | 22.43 | 22.97 | 19.03 | 22.09 | 20.78 |
| reasoning | 26.92 | 27.84 | 28.32 | 60.77 | 58.39 | 44.50 |

Review prefill was 293.89 → 281.23 → 386.68 tok/s for original, drafting only and combined candidate, respectively. The corresponding TTFT means were 21.28 s / 22.24 s / 16.17 s. The source candidate therefore improves review prompt processing by about 32%, while review decode is essentially unchanged. Editing decode improves about 94% through configuration alone and about 106% with the combined candidate; the latter adds about 6% over drafting only.

Full-answer wall time must be interpreted alongside output length. Candidate review answers were 787/841 words versus 927/938 for the original; candidate reasoning was 909 words versus 1,040. Prose was 577 versus 517 words. The six cached-conversation answers all hit the 1,800-token cap, so they do not establish full-answer completion.

### Final near-limit context comparison

The last pair used the same 84,859-token prompt, unchanged 90,000 requested context, and the same 203-token answer. Both passed strict JSON and every expected field.

| Metric | Original | Combined candidate |
|---|---:|---:|
| Prefill tok/s | 281.81 | 395.38 |
| Decode tok/s | 20.47 | 23.32 |
| First output s | 301.14 | 214.65 |
| Complete response s | 311.01 | 223.31 |

The combined candidate reduced the request from 311.01 to 223.31 seconds (28.2%), with about 40% faster prefill and 14% faster decode. Unlike the earlier mask-only test, the private pinned loader also reduces the dominant prefill time. This final combination has one long-context pair; the six earlier runs separately support the masked-attention component’s decode benefit. It is not a broad long-context quality benchmark. See final long-context quality and raw summary.

### Startup and memory in the final full-answer runs

At readiness, original private anonymous memory was about 380 MiB and drafting-only about 395 MiB, versus 34,804–34,850 MiB for the candidate. Total RSS was similar because original memory was predominantly file-backed. Candidate process swap was approximately 55–99 MiB at readiness; the original/control samples showed zero. CUDA-registered memory is not represented as Linux `Locked` bytes in these smaps records. Peak process-specific GPU memory was 13,678 MiB original, 13,718 MiB drafting-only and 13,440 MiB candidate. This small VRAM reduction does not offset the substantial change in host-memory commitment.

### Answer quality

All six source-edit outputs were byte-identical and correct after trimming outer whitespace, but omitted the required final newline. All six repetitive rewrites were also byte-identical and content-correct after stripping the unwanted fence. Both remain strict-formatting failures. The tool-shaped JSON fixture parsed in all six runs but differed from the frozen expected schema; this fixture has a documented wording/schema ambiguity. Separately, all six native function-call smoke checks passed, while their direct-answer JSON checks failed because of an outer Markdown fence.

The six code reviews cover the main architecture but overstate some guarantees and omit important custody/readiness details. Both original and candidate prose omit important cancellation/committed-effect qualifications. Both crash-recovery designs have substantive durability and replay flaws; the candidate includes internally inconsistent recovery rules. The measured execution gains use the same stored model, but this small sample does not establish unchanged reasoning quality or bit-exact generation. Review answers also differed between repetitions within each arm. No prompt or scoring criterion was altered to rescue a failure. See mechanical checks, predeclared rubric and answer-specific manual assessment.

## Revisions under test

| Family | Pinned source revision |
|---|---|
| Original Unsloth |92cedc8679d145902ead3f006258e8672eac11e6|
| Newer Unsloth b10909-mix-bea84f7 |329b6160f513915f1c607dbfae3d5ce864a64a4f|
| Upstream llama.cpp |97e4ca73582084f2751767f80c86237483ecc381|
| TheTom TurboQuant fork |407f3237bfb3eeaff61546797de3d8c1a96be748|

The newer source families were built separately for the target GPU architecture. Runtime build strings from archive builds may lack git metadata; the pinned source hashes and build manifests are authoritative. The old production source/installations were never used as mutable experiment trees.

## Reproduction

See REPRODUCTION.md for the methodology. Source patches, compatible build inputs and a separately reviewed evidence package are required to repeat the exact experiment.
