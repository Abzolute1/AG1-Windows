# TurboQuant 407f323 Flash-Next source review

Experimental source: `TheTom/llama-cpp-turboquant` revision `407f3237bfb3eeaff61546797de3d8c1a96be748`. This is an inference-runtime fork, not a model copy. All experiments reuse the same preserved 28 GGUF shards. The older Qwen3.6 tuned runtime already originated from an earlier revision of this general fork; no speed claim for the newer fork on Qwen3.6 has been established.

## Relevant differences before measurements

- Hybrid expert cache supports this model's MXFP4, IQ1_M and IQ2_S types. GPU-resident hits can run alongside CPU misses; background admission uploads selected experts. This is independent of learned PLE n-gram embeddings and optional speculative drafting.
- Ada's default minimum expert size is 512 KiB. Middle-layer IQ1_M gate/up experts are approximately 350 KiB each and excluded by that threshold. Testing 256 KiB admission changes only cache eligibility, never quantization or model mathematics.
- Automatic overlap calibration excludes prefill, rotates candidate order across rounds, and times single-token decode scopes. Warm-up/calibration and cache-history effects must be reported separately from settled throughput.
- Unlike preserved b10715 and newer Unsloth b10909, this revision creates the PLE tensor with flags 0 rather than `TENSOR_READ_LAZY`. It has lazy-loader infrastructure but does not opt this model's huge PLE table into it. First unmodified loading is being measured; repeated eager reads may cause avoidable load/cache-pressure costs.
- Its PLE input maintains a per-sequence token-history snapshot, while b10715/b10909 obtain predecessor tokens from KV cells. Its PLE graph input lacks the corresponding `can_reuse` override. Cached continuation, state recovery, speculative rollback, and graph reuse need explicit validation.
- The newer Unsloth source changes GDN L2 normalization to scaled RMS normalization with epsilon inside the square root (PR 28068). Original b10715 and this fork use `ggml_l2_norm`. This is a numerical/semantic compatibility consideration when comparing builds, separate from speed.
- The fork's MTP graph differs from newer Unsloth in head sharing. Existing sidecar compatibility cannot be inferred merely from architecture support.

No production paths or model bytes were modified. No fork performance result was available when this note was created.

## Stock screen allocation findings

The 4480 MiB cap allocated3214+985=4199 MiB of expert pools; the6000 MiB cap allocated3202+981=4183 MiB. Both were limited by the default3072 MiB reserve, so this is not evidence that increasing actual cache capacity improved speed. The6000-cap run chose a different overlap calibration and showed about24tok/s review versus~22 for4480; this must be disentangled from run variation and calibration. Reserve1024 is a separately queued variable. Final hit rates were32.8% and33.5%, with254,045 and252,708 evictions respectively. The caps must not be described as allocated cache sizes.

## Explicit overlap sweep interpretation

The explicit0/4/8row overrides are honored: the configuration log records the requested value and the source bypasses automatic calibration when it is explicit. However, `moe_cache_plan` applies forced CPU overlap only when **all selected rows are cache hits**. Partial-hit nodes already send their misses to the CPU and do not use this override. This explains the limited impact on the poorly cached exact-edit workload; similar results do not mean the environment variables were ignored. The completed larger-cache run's cumulative hit fraction fell from over60% early to about40% after edits, with substantial eviction counts. These are cumulative expert-cache statistics, not speculative token acceptance rates.
