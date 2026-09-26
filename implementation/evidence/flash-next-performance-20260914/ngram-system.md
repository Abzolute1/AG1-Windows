# Flash-Next n-gram subsystems: implementation map

This investigation distinguishes two mechanisms that have different correctness and performance properties.

## Learned PLE in the target model

The exact GGUF reports qwen4exp with one PLE layer, bigram/trigram hashes, eight heads for each n-gram length and160 dimensions per head. The immutable per_layer_token_embd table contains320001536 rows in Q5_1, occupying35.763GiB. Sixteen selected rows contribute to each input token. This is trained model computation, not a draft-token predictor, and cannot be disabled or replaced with a smaller table while claiming unchanged semantics.

The host constructs row indices from token IDs and predecessor history, respecting sequence positions and EOS boundaries. The newer Unsloth source uses an indexed predecessor lookup. Row gathers decode the selected quantized rows; small resulting activations feed the model graph. Most of the35.76GiB table remains file-backed and demand-paged in the original setup.

Each Q5_1 row is120 bytes, so the raw selected weight payload is1920 bytes/token. Page faults can move much more because the operating system works in pages and may perform read-ahead. This is why page residency, actual I/O, file backing and warm/cold state must be distinguished from tensor size. The machine stores these files on a SATA SSD.

Unsloth prefetch_rows issues memory advice for merged selected page ranges before execution. It is not a persistent CPU candidate-generation thread overlapping speculative verification. The isolated direct-read port uses buffered pread into gathered row storage and creates workers for large gathers; it is not O_DIRECT and does not change Q5_1 decoding. Parallel mmap gathers and direct reads did not show a meaningful warm improvement in the completed screens. Three balanced PLE-cold runs per arm likewise did not show a benefit from the parallel gather patch.

There is no PLE draft acceptance rate: the selected embeddings are model inputs, not proposals to accept or reject.

## Optional runtime ngram-mod speculation

The original launch enables no speculative implementation. When explicitly enabled, common/ngram-mod.cpp maintains a separate CPU dictionary of4194304 int32 entries,16MiB. For a matching prefix (24 tokens in the planned screen), it computes a polynomial/LCG-style hash modulo the table size and stores the following token. The entry contains the token, not a full collision-proof key. False proposals still need target verification.

The current implementation recomputes the hash for each lookup; it does not implement the full rolling-hash recurrence. Candidate generation follows stored successor tokens until a miss or the configured maximum depth. Drafts shorter than the minimum are discarded. The dictionary is shared across server sequences and survives begin() calls, so independent cold-dictionary comparisons require new server processes. KV cache_prompt=false alone does not reset it.

The target verifies proposed tokens together. common_sampler_sample_and_accept_n samples from each target-model position, commits that sampled token, and stops on the first mismatch with the proposal. A completely accepted draft also yields the next target token. Rejected suffixes require attention/recurrent/PLE state rollback; this architecture makes rollback capability and allocation part of the performance and correctness investigation.

Five consecutive low-acceptance rounds (fraction below0.25) reset the dictionary. Occupancy above0.25 also resets it at request start. The source warns against match lengths below16; the initial screen uses24. A theoretical cheaper hash is not automatically a useful optimization: CPU lookup cost must be measured against target verification and memory-transfer costs before changing data structures.

Actual native final timing records expose draft_n and draft_n_accepted when drafting occurs. These, token throughput, latency and correctness are recorded separately for source editing, repetitive code, prose and free-form reasoning. A target-verified proposal is intended to preserve the target distribution; differences in batched floating-point execution and state rollback still require empirical correctness checks.

MTP is a third, optional learned drafting mechanism. Existing Q4_K_M and Q8_0 MTP sidecars have been inventoried and hashed separately; they are not enabled in the original launch. Their quality/acceptance and memory costs must not be attributed to the built-in PLE table.

## Local source references

- Original exact source: /home/alex/Projects/flash-next-perf-20260914/original-source/llama.cpp-b10715-mix-86bd2d3
- Newer exact source: /home/alex/Projects/flash-next-perf-20260914/unsloth-new-source/llama.cpp-b10909-mix-bea84f7
- Architecture and PLE graph: src/models/qwen4exp.cpp and graph-input/history helpers.
- Optional dictionary: common/ngram-mod.cpp and common/ngram-mod.h.
- Candidate lifecycle: common/speculative.cpp, common_speculative_impl_ngram_mod.
- Verification sampling: common/sampling.cpp, common_sampler_sample_and_accept_n.
- Commit/rollback: tools/server/server-context.cpp speculative acceptance path.

[Upstream ngram-mod proposal and discussion](https://github.com/ggml-org/llama.cpp/pull/19164), [direct PLE read proposal](https://github.com/ggml-org/llama.cpp/pull/28136). The numerical observations above are from local evidence, not claimed upstream benchmark results.
