# MoE graph-memory review

Source: `/home/alex/Projects/llama-perf-20260914/original-experimental`, revision `bd1bf025fc55ffa1fcb2ba6d8bb8805f35671d1f`. This was a read-only review; no GPU runs, builds or implementation edits were performed. All references below are relative to that tree.

## Main finding

There is **no demonstrated 64/128 MiB saving from changing expert weighting to `ggml_mul_inplace`**. The allocator already allows ordinary MUL and ADD outputs to reuse dead input storage (`ggml/src/ggml-alloc.c:22`). At lines 630-678 it reuses an owned, same-layout parent with one remaining child and no other views, unless that parent or its view base is marked as an output. Consequently, the plain `ggml_mul` at `src/llama-graph.cpp:2267` can already reuse the down-projection result under the ordinary eligible graph conditions.

Likewise, the seven ADD nodes in the eight-expert prefill reduction do not imply seven simultaneously allocated outputs. The first ADD produces a contiguous `[2048,B]` result, and subsequent ADDs can reuse that result. At B1024/B2048, the expert slab is64/128 MiB and one reduction result is8/16 MiB, but actual peak changes depend on graph lifetimes, backend copies, other concurrent tensors and allocator reuse. These are logical payload sizes, not measured VRAM savings.

No proposed mutation below is ranked as a proven memory improvement. The safe next step is an allocation/lifetime trace of the existing graph at the requested batches, preserving backend assignment and showing whether the suspected slab duplication actually exists.

## Expert weighting: dataflow and aliasing

`build_moe_ffn` obtains `experts` from the down projection (`src/llama-graph.cpp:2253`), optionally applies down bias, then applies routing weights when `weight_before_ffn` is false. Within this helper there is no later mathematical consumer of the unweighted final `experts` value; the remaining consumers are views/reduction of the weighted result. This supports the algebraic idea of destructive weighting for a private intermediate.

However, `ggml_mul_inplace` creates a view of its first operand (`ggml/src/ggml.c:2300`), rather than merely asking the allocator to try reuse. `ggml_view_tensor` retains source strides and a view relationship (`:2004`). That changes graph ownership, scheduling constraints and lifetime information. Ordinary `ggml_mul` leaves these choices to the allocator, which can decline reuse when it is unsafe.

LoRA/scales/biases do not make the final result an original weight tensor: `build_lora_mm_id` creates a matmul result, optional scaling and adapter additions (`src/llama-graph.cpp:1541`), and down bias is added before routing weighting (`:2261`). Nevertheless, proving private ownership requires checking the *actual final graph*, including aliasing and consumers introduced by adapters, graph callbacks or future extensions. A generic in-place change in shared `build_moe_ffn` would also affect architectures beyond this Qwen workload, including the Metal rescale path and `weight_before_ffn` alternatives.

The normal runtime graph callback names tensors and sets selected `norm`/`l_last` backend assignments (`src/llama-context.cpp:2778`); it does not by itself create a persistent consumer of `ffn_moe_down`. The generic `llm_graph_context::cb` dispatches an arbitrary graph-building callback (`src/llama-graph.cpp:1496`), so this normal behavior must not be assumed for every caller. Separately, evaluation callbacks can inspect intermediate tensor values during execution. Keep their observation order intact, and do not overwrite intermediates marked as graph outputs or retained for a diagnostic consumer.

### Cross-device constraint

The target keeps some expert weights on CPU. That does not guarantee that down projection, routing weighting and reduction all execute on the same backend for every batch. The scheduler derives locations from weights, supported operations and transfer decisions. `view_src` affects backend selection (`ggml/src/ggml-backend.cpp:1075`), and remaining views are assigned to the source backend (`:1414`, `:1427`). Split-input dependencies are explicitly added to protect source/copy lifetimes (`:1628`).

An explicit view-backed MUL can therefore constrain where weighting executes, change a split or require a different transfer. It is not safe to infer unchanged GPU allocation from local tensor shape alone. Before even a Qwen-only experiment, prove identical CPU/CUDA assignments and transfer edges at decode, B1024 and B2048, plus source/output buffer compatibility. Reusing the allocator's existing automatic ownership checks is preferable to bypassing them.

## In-place addition into an expert view

The prefill reduction creates eight strided expert views of one slab (`src/llama-graph.cpp:2296`). Each view has one token-row stride equal to the entire eight-expert slab's token stride. The ordinary first ADD returns a newly contiguous tensor because `ggml_dup_tensor` creates a fresh tensor with standard strides (`ggml/src/ggml.c:1936`, `:2099`). Subsequent contiguous ADD results are eligible for automatic storage reuse.

Writing the sum into expert slot zero can preserve the same left-associative arithmetic if every operation reads its operands before writing disjoint output elements. But it changes output layout and retains the whole base slab as the accumulator's owner. The allocator tracks base-view lifetimes (`ggml-alloc.c:739`, `:806`). A surviving slot-zero output can therefore keep64/128 MiB alive where a separate8/16 MiB contiguous result previously let the slab die. Qwen's result survives while shared-expert FFN/gating runs before the final addition (`src/models/qwen35moe.cpp:526`, `:547`), making this lifetime extension particularly relevant.

CUDA ADD/MUL advertises supported F32/F16 operands (`ggml/src/ggml-cuda/ggml-cuda.cu:5240`), and the broadcast implementation carries strides (`ggml/src/ggml-cuda/binbcast.cu:277` onward). That does not prove an arbitrary overlapping strided write is safe across every fusion and consumer. Full stride/overlap checking, backend assignment and callback semantics would still be needed. Adding a final contiguous copy could restore layout and release the slab sooner, but also reintroduces an8/16 MiB result and adds work. No net benefit is established.

**Recommendation:** do not implement in-place accumulation into expert views as a presumed memory optimization.

## Does backend fusion eliminate intermediate allocations?

Execution-time fusion can skip writing intermediate nodes while their graph allocations remain reserved. The scheduler optimizes each split before constructing the allocator graph (`ggml/src/ggml-backend.cpp:1624`), but the CUDA runtime also recognizes fused sequences during graph evaluation/capture and returns a count of nodes to skip. Its memory-overlap guard operates on already allocated addresses (`ggml/src/ggml-cuda/ggml-cuda.cu:3020`). Thus execution fusion alone is not proof that intermediate allocations disappear.

This does not justify deleting intermediate nodes from the allocator graph independently. Fusions are conditional on shapes, backends, memory overlap, options, callback execution and graph reuse; the unfused fallback must remain executable. A planner-level replacement would need an explicit fused operation with a known scratch/output contract and fallback implementation, or a backend optimization API that returns the transformed graph before allocation and supports all runtime modes. That exceeds a simple model-graph alias edit.

Model-level use of existing operations can preserve weights but still change summation order, tensor layout and temporary memory. For example, applying the decode `permute -> cont -> sum_rows` route to prefill materializes another expert-sized slab and changes the reduction path; the existing source explicitly keeps the view/add route for prefill because of its performance cost (`src/llama-graph.cpp:2276`). A matrix-product weighted reduction could change arithmetic/accumulation behavior and needs its own correctness and memory analysis. Neither is a proven-safe memory win from this review.

## Ranked safe action and acceptance constraints

1. **Trace existing allocation reuse before changing the graph.** In a separate diagnostic build or allocator trace, record names, source/view owners, child/view counts, backend IDs, allocated address ranges and peak live allocations for `ffn_moe_down`, `ffn_moe_weighted`, expert views and ADD results. Confirm whether the weighting output shares storage, whether the ADD chain reuses its first contiguous result, and how long the expert slab survives across shared-expert work. Compare B1024 and B2048. This is the only presently justified action; there is no proven implementation candidate to rank above it.

If the trace reveals a real missed reuse case, constrain any follow-up to the exact Qwen graph and explain why the normal allocator could not safely reuse it. Require private owned intermediates, no extra consumers/output flags, valid nonoverlapping operands, identical backend placement/copy dependencies, supported output strides, preserved rounding/order and unchanged callback observations. Do not weaken allocator ownership tests merely to reduce reported VRAM.

For a model-graph-only patch using existing operations, validate the full graph on CPU and CUDA with decode, both prefill batches, mixed CPU/GPU placement, no adapters and adapter/scale/bias negative/fallback cases; include exact output comparison where operation order is unchanged, real Qwen correctness tasks, graph reuse, cancellation and measured memory peaks. If the solution touches allocator/backend/quant code, the original `AGENTS.md` backend gates apply: `test-turbo-quant`, `test-quantize-fns`, a full actually executed CPU+CUDA `test-backend-ops` sweep and the documented `llama-bench` coverage. Final acceptance requires repeated actual inference and memory measurements, not tensor-size arithmetic.
