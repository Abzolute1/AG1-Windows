# Fork PLE rollback correctness review

Source: TheTom/llama-cpp-turboquant 407f3237bfb3eeaff61546797de3d8c1a96be748, src/models/qwen4exp.cpp and src/llama-memory-hybrid-idx.cpp.

The fork retains only the last `ple_ngram_size - 1` tokens per sequence (two for this model). `ple_hist_truncate()` clears that history when rewinding to or before its beginning. It cannot reconstruct the two predecessors at the rewind point. `set_input()` then EOS-pads the missing predecessors. This is a concrete source-level concern for partial prompt-cache reuse and speculative rejection; ordinary uninterrupted decode does not trigger this particular case.

Example: after tokens [10,20,30,40,50] at positions 0..4, history is [40,50], next_pos=5. Rewinding to position 3 clears the stored history. The next hash at position 3 should use predecessor tokens [20,30], but this history representation has discarded them. Rewinding by one token also leaves only one predecessor instead of two.

The original and newer Unsloth implementation obtains predecessor tokens from cached KV cells instead. A server may restore an earlier full checkpoint or reprocess enough tokens in some cases, so source analysis alone does not establish the frequency of wrong live outputs. Test cached continuations, partial-prefix changes, and speculative rejection explicitly. Do not accept a throughput gain as evidence of correctness.

The current lazy-loading/graph-reuse patch does not change this history representation. Cache and speculation screens are exploratory, not an approved production recommendation. A repair must preserve retained predecessors through supported rewinds and state serialization, and must be tested against a fresh-prefix reference before recommendation.

## Actual server diagnostic qualification

The completed original, lazy/reuse fork, and retained-history fork diagnostics all kept the same greedy next token in six fresh/append/rewind cases. Retained history produced exactly the same teacher-forced probability records as the lazy/reuse fork in the tested32-token prose/edit prefixes. The fork differs slightly from the original numerical execution (30/32 prose greedy matches,32/32 edit matches; coarsened top32 KL about0.00194/0.00043), which is not by itself an intelligence regression.

Crucially, the rewind-one case **does not exercise a direct PLE history rewind**: the server log restores a checkpoint ending at token897, then re-evaluates three tokens. That checkpoint already contains the required last two predecessor tokens. Rewinds4/32 re-evaluate the whole900-token prompt. The identical retained/unretained rewind result is therefore expected; it does not demonstrate that the retained-history code failed. Fresh versus chunked prompt evaluation also differs numerically in the preserved original.

The source counterexample remains relevant to **direct recurrent-state rollback without checkpoint restoration**, such as a future ngram/MTP path using retained recurrent snapshots. The stock fork's currently tested ngram-mod path instead uses checkpoint restoration when recurrent snapshots are unavailable. Do not describe the source concern as a demonstrated current AG1 corruption bug, nor the retained-history overlay as a measured performance improvement. Direct-rollback validation would be needed before enabling that additional execution strategy on the fork. Raw logs: `runs/turbo-history-cacheoff/server.log`, especially the checkpoint restoration at n_past897.
