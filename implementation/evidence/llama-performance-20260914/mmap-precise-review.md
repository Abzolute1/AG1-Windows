# Precise mmap registration shim v2: read-only review

Reviewed `/home/alex/Projects/llama-perf-20260914/mmap-precise.cpp` as supplied, against original experimental llama.cpp revision `bd1bf025fc55ffa1fcb2ba6d8bb8805f35671d1f`. Shim line references refer to the 186-line version inspected here; later edits may move them. No build, model execution or GPU workload was run by this review.

## Assessment

V2 fixes the specific ordering that exposed initial GPU uploads to partially registered source pages: it records CPU tensor page intervals during allocation and registers them only after the original `llama_model_load_from_file` returns successfully (shim112-145). The inspected original load function waits for mmap CUDA copies and tensor validators before success. This is a meaningful correction, but it does not establish general lifetime safety or guarantee that every later runtime DMA source is fully registered.

The shim is reasonable to investigate under a tightly bounded, single-model, sequential process after addressing incomplete registration and documenting cleanup. It is not ready to become a general installed runtime feature. No performance or stability improvement is claimed here.

## Verified protections

- The mmap interception requires the exact configured path, a read-only file descriptor, offset zero, readable protection and no existing write protection (shim75). The selected mapping becomes `MAP_PRIVATE` and writable (79). CPU or device writes to that private mapping cannot write back to the original file. This preserves the file, but does not make in-memory values immutable.
- Registration is based on actual CPU tensor extents, page-rounded outward (99-109), rather than the entire minimum-to-maximum CPU context span. This avoids pinning all GPU-only holes between CPU tensors when interval accounting succeeds.
- `mapped_range` checks continuous VMA coverage and exact path/private-map attributes, not just the first address (43-56). It therefore rejects unmapped holes produced by loader trimming. It does not verify a file offset/device/inode identity or that every accepted VMA remains writable; those are additional robustness requirements for a general integration.
- Original successful mmap uploads call synchronous `ggml_backend_tensor_set`; CUDA's implementation waits on its copy stream before return (`ggml/src/ggml-cuda/ggml-cuda.cu:855`). Original tensor validators are joined in `src/llama-model-loader.cpp:1667` before successful loading finishes. Deferring registration until success therefore prevents the v1 load-time boundary case.
- Original failure/cancellation destruction order is loader first in construction, model second (`src/llama.cpp:307`, `:311`), hence model buffers free before loader mappings. Successful model destruction likewise frees `ctxs_bufs` before mappings (`src/llama-model.cpp:1032`, `:1039`). Pending, unregistered spans need no CUDA teardown on a failed load.
- V2 now calls original `ggml_backend_buffer_free` before unregistering (shim177), so the fork can invalidate and drain that buffer's MoE-cache source users first (`ggml/src/ggml-backend.cpp:219`, `ggml/src/ggml-cuda/moe-cache.cu:3020`). The verified CPU buffer-from-pointer callback does not free the underlying mapping (`ggml-backend.cpp:2591`), allowing this narrow order.

## Material correctness/lifetime holes

### 1. Partial registration can recreate a runtime boundary problem

Failures at shim140 are logged while earlier successful registrations remain active. Skipped intervals at108/132 likewise leave no explicit completeness record. The model can therefore run with a mixture of pinned and unpinned pages. Registration after loading protects initial uploads, but CPU-resident expert tensors may later be copied to CUDA during prefill or cache population. A source range crossing a registered/unregistered boundary can reproduce the class of error observed in v1.

For a controlled experiment, require complete coverage of every intended runtime DMA source tensor, including scheduler copy padding, or roll back all registrations for that model before returning it. Log requested CPU-tensor bytes, page-rounded required intervals, successful coverage, skipped ranges and failure status. A partial run must not be labeled equivalent to the intended fully registered variant. The existing successful-range log alone is insufficient.

### 2. Overlapping owners lack shared registration lifetime

`uncovered` excludes intervals already registered by any owner (58-69), but the second owner does not record a reference to that registration. If two CPU wrappers share a page, freeing the first unpins memory still used by the second. Their DMA calls could then cross a mixed boundary. The exact single-file/single-CPU-wrapper experiment may avoid this, but that is a scope assumption to assert, not a property of the registry design.

For general use, maintain a model/mapping-owned interval registry with shared reference counts or keep all registrations until every referencing buffer is drained. Registration owners should not depend on nondeterministic unordered-map iteration when overlapping lifetimes matter.

### 3. Teardown removes protection from the registry too early

At175, spans are erased from the global registry before original buffer free drains users and before CUDA unregistration completes. A concurrent release hook at164 then treats those still-registered ranges as uncovered and can call `MADV_DONTNEED` on them. A mutex around map access does not close this lifetime window.

Keep a retiring registration visible to exclusion checks until drain and unregistration finish, or prohibit/serialize release and model teardown explicitly. Avoid holding a global mutex through operations that can reenter the hooks without an established lock-order design. Sequential one-model benchmark shutdown may avoid the race but does not prove production safety.

### 4. Failed unregistration is not successful cleanup

At182, failure is merely logged and ownership is already erased. Mapping destruction can follow without successful unregister. Treat any unregister failure as a stability failure in evidence. A proper integration needs an explicit recovery/ownership policy and must not silently report clean teardown. CUDA APIs can also surface earlier asynchronous errors; clearing or ignoring such errors should not turn an invalid inference run into a valid benchmark sample.

### 5. Global mapping state has only one slot

`map_begin`/`map_end` describe only the most recently selected mmap, are published as two independent atomics and are never cleared on unmap. Registration iterates all owners after an exact-path model load rather than owners attached to that returned model. Multiple simultaneous or repeated same-path model loads, alternate mapping strategies, or address reuse can associate intervals with the wrong generation. `mapped_range` mitigates unmapped/address-path mismatches but does not establish model identity or atomic range publication.

Constrain the experiment to one selected mapping and one model load per process, assert it, and make the environment immutable. A lasting implementation should use per-mapping identity and model-owned RAII. The `llama_model_load_from_splits` entry point is not intercepted; common original single-file loading does call the hooked API (`common/common.cpp:1298`), but other entry points must not be assumed equivalent.

### 6. Exception and range bookkeeping needs hardening

The successful-registration `push_back` is protected by a prior reserve, which improves rollback safety, but sorting/merging/uncovered allocations or registry insertion can still throw. At post-load registration time the returned raw model pointer has no new RAII owner in the shim. An exception can escape after some registrations succeed and before the caller receives the model. Add rollback and model cleanup, or make the optional registration phase catch allocation failures and return a fully reverted ordinary model.

Check `sysconf(_SC_PAGESIZE)` before unsigned conversion, bound pointer additions and page-rounding, and validate nonzero tensor sizes before rounding. Current normal Linux address ranges make overflow unlikely, but unchecked arithmetic is not a reusable registration API. The end-of-file page can round beyond `map_end` and be skipped even though its valid bytes are mapped; record such coverage loss and handle the final partial file page deliberately, never by reading past EOF.

## Optional MADV_DONTNEED hook

The hook first performs the original synchronous copy, then advises only inward-rounded full pages (148-165), verifies the source mapping, and excludes known active registered intervals. It does not unmap virtual addresses. This is safer than early `munmap` for outstanding CPU validators or later aliases: readers can fault unchanged file-backed data back in.

However, `MAP_PRIVATE|PROT_WRITE` means pages could acquire private modifications. `MADV_DONTNEED` discards those modifications and later reloads file bytes. Its content-safety proof therefore requires that eligible pages contain only the original immutable GGUF values. The shim itself does not perform numerical writes, but it has removed the original mapping's write protection. Do not enable this optimization alongside in-place weight transforms or other writers. CUDA registration can create private page residency without a numerical change; that is a memory-cost issue, while numerical private modifications would be a correctness issue.

Pending future CPU registration spans are not excluded while loading. Advising their clean pages before they are pinned does not change model values, but can force them to refault and erase the expected memory/load-time benefit. Concurrent validation can also refault advised pages. These are performance confounds rather than an inherent content race when backing bytes remain immutable. Existing mlock or third-party registrations are not tracked by the shim; exclude those modes or integrate their interval ownership.

Device name `CUDA0` is a narrow heuristic, not a general proof that a backend's `set_tensor` is synchronous. It is justified only for the inspected original CUDA implementation and unchanged binary/library selection. Keep that exact-version restriction. Record failed advice calls; `discarded_bytes` is a cumulative sum of requested successful advice lengths and may count the same page repeatedly. It is **not** unique released bytes, lower RSS, or freed system RAM.

Removing `MAP_POPULATE` does not suppress the original constructor's later `POSIX_MADV_WILLNEED` request (`src/llama-mmap.cpp:463`). Thus v2 changes immediate population but not all read-ahead behavior. Measure actual process RSS/VmHWM, page faults, system available memory and load/first-inference timing. Writable CUDA registration may materialize private copies and increase anonymous/private RSS; report that tradeoff instead of assuming mmap implies shared resident storage.

## Required evidence before accepting a variant

For the bounded single-model experiment, record exact shim/binary/library identities, requested mapping path/device/inode, original model hash from the preservation audit, mapping protection/flags, full required and successful registration coverage, total unique pinned bytes, absence of registration/unregistration errors, and model-load/teardown completion. Test both release-disabled and release-enabled cases separately.

Require repeated real prefill/decode at the target context and batches, enough routed expert use to exercise boundary tensors, deterministic task correctness, and clean teardown. A complete successful registration phase does not alone validate every later expert copy. Include load cancellation and a forced registration failure/rollback case before considering installation. For concurrent/multi-model integration, fix shared ownership and retiring-range handling first rather than relying on the single-GPU benchmark process model.

The original model file remains protected by the read-only FD and private mapping. The outstanding concerns are in-memory value/lifetime correctness and honest memory accounting, not a demonstrated file-write path. No production integration is recommended from source inspection alone.

## Follow-up: unbuilt v3 source review

Also inspected `/home/alex/Projects/llama-perf-20260914/mmap-precise-v3.cpp` without building or running it. This follow-up supersedes the relevant v2 findings only where explicitly stated.

### Corrected by v3

- The registry now has one shared registration owner. All collected owner intervals are merged before pinning, and registration persists until every tracked CPU wrapper has passed through original buffer free. This corrects the v2 missing shared-page ownership problem within the tracked single-model lifecycle.
- Original buffer free now runs while registration spans remain visible globally. Removing the final wrapper, unregistering and clearing spans happen under the same registry mutex used by the release hook. This closes the specific v2 erase-before-drain/unregister release window.
- A reported registration API failure or a failed post-load merged-range VMA validation causes rollback of previously successful registrations. Unregister errors terminate the experimental process instead of continuing to normal unmapping. This is an appropriate fail-closed experimental behavior; the sample is failed, not a usable fallback benchmark.
- Reserving successful-span storage before registration makes each subsequent successful-span insertion nonallocating. That is a useful part of transactional exception safety.

### Remaining completeness hole

`ggml_backend_tensor_alloc` still silently skips a tensor if its outward-rounded page span fails `mapped_range`. Wrapper provenance failures likewise produce no owner/collection failure record. Those omissions occur **before** the post-load transaction builds its merged required intervals. The final `complete=true` therefore means all *collected* intervals registered, not all eligible CPU tensors were covered. An empty collected set can also report complete.

This matters for a final partial file page: `inside` bounds checks against the original byte length while outward rounding extends to an OS page boundary. A valid tensor ending in that page can be skipped. If a neighboring tensor pins part of the skipped tensor's source range, later DMA can again encounter mixed registered/unregistered pages. Other provenance/alias paths may have the same consequence.

Add an explicit collection-completeness state and audit the union of registered pages against every intended mapped CPU tensor's valid byte interval, including any extra bytes copied by the scheduler. Record expected and observed tensor counts, valid-byte totals, page intervals and every skip reason. Any eligible tensor omitted from collection must disable/roll back the entire registration variant or terminate the experimental run. Treat mapping byte bounds and OS-rounded VMA coverage separately; never solve this by reading invalid tensor bytes past EOF. A normal single CPU owner does not by itself prove completeness.

### Remaining exception safety

The post-load transaction performs `mapped_range` checks inside the loop after earlier pins may have succeeded. Those checks allocate C++ stream/string objects and can throw; there is no enclosing exception handler that rolls back existing pins. Gathering/merging intervals and registry insertion can also throw after the original load returns a raw model pointer, which the wrapper does not protect with a temporary owning guard.

For the bounded one-mapping/no-concurrent-unmap experiment, prevalidate all merged ranges before the first registration, reserve ownership storage, and still catch failures around the complete registration transaction. Roll back successful pins on failure; if cleanup fails, retain the experimental `_exit` policy. If returning a fallback model, ensure there are zero remaining pins and the model pointer is neither leaked nor freed twice. The registry/tensor-allocation hooks also need an explicit failure policy rather than allowing arbitrary C++ exceptions to escape an extern-C API boundary.

### Remaining scope limits

The simultaneous-load rejection checks whether shared registration spans already exist only after another original model load has completed. It is not an atomic guard against a second mapping overwriting global `map_begin`/`map_end` during loading; if registration was empty or rolled back, it does not identify another live model either. Continue to restrict the experiment to one primary mapping/model load per process and immutable configuration. A robust single-load guard belongs before the selected mapping/loading work, with an explicit loading/loaded state rather than using nonempty pin storage as a proxy.

Original buffer free deletes the wrapper before v3 takes the registry lock and erases its pointer key. With concurrent unrelated wrapper allocation and address reuse, that key could be reused during the gap. The current sequential, single-model teardown avoids this, but a general implementation needs stable owner identity or teardown state, not allocator pointer identity alone.

All earlier COW-content assumptions, MADV_DONTNEED accounting caveats, remaining WILLNEED read-ahead, exact CUDA binary restrictions and required benchmark/correctness evidence still apply. V3 is a material improvement to the experiment's ownership design, but `complete=1` is not yet sufficient proof of full runtime DMA coverage.
