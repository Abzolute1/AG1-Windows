# Completed Flash-Next investigation

**Final status: completed 15 September 2026.** [Report](REPORT.md), [completion record](final-closeout.json), [preservation audit](final-preservation-audit.json), [AG1 scores and limits](ag1/scored-results.json), [reproduction archive](reproduction-scripts-final.tar.gz). No active inference/desktop process or further test queue; no production cutover. Historical checkpoints below are superseded.

# Flash-Next investigation — active

Started 14 September 2026. User-authorized autonomous performance engineering; no production cutover authorized without evidence. Original runtime/configuration/model files remain read-only. All experiments: `/home/alex/Projects/flash-next-perf-20260914`.

## Verified setup

- AG1 production runtime: `/home/alex/Projects/llama-cpp-qwen4exp-mtp-b10715/llama-server`, Unsloth build10715, commit92cedc867.
- Exact model: Qwen3.8-Flash-Next-AD-3.84bpw-IQ4_XS-M64, 28 shards, 79.10GiB total file bytes. Existing UD label does not describe the active profile.
- Effective context90000, batch1024, microbatch1024, CPU MoE layers42, automatic GPU layers, Q8_0 K/V, FlashAttention on, 12 decode/24 batch threads, strict CPU ranges0–11/0–23, poll100. Exact argv/environment: original-launch.json.
- No speculative mode enabled in original configuration. Built-in PLE is a learned bigram/trigram hash embedding mechanism, distinct from runtime n-gram speculative drafting.
- PLE table: 320001536 rows ×160 dimensions, Q5_1, 35.76GiB lazy mapping. 16 hash-selected rows/token. Model uses48 layers,512 experts/10 active and hybrid gated-delta/sparse attention.
- All28 model hashes matched sealed AG1 capture on initial inventory pass; see inspection.log. Protected runtimes/configs hashed in protected-before.json. A runner import initially shadowed Python inspect with inspect.py and unintentionally repeated read-only inventory; it was stopped and renamed inspect_original.py before inference. Partial repeated inventory is retained separately; canonical original model manifest reconstructed from the hashes verified by the completed initial pass.

## Baseline

Runner bench.py adapted from prior35B runner; new frozen Flash-Next prompts cover A–H. Original settings retained except private HTTP port18438. Greedy controlled comparisons, three repetitions/class, native streamed completion timings and half-second process/GPU/disk-I/O telemetry. Warm cached conversation is separately labeled; fresh requests explicitly disable prompt cache. Actual AG1 desktop comparisons follow promising candidates.

Initial original baseline completed:24 requests, three per A-H category. See run-summary.json and quality-adjudication.md. Warm review prefill302tok/s / decode28tok/s; long21k prefill305tok/s / decode26tok/s; exact file editing passed3/3 at29tok/s. First-use load85.4s versus subsequent cached starts3.6-5.4s. Seven configuration discovery profiles completed. Smaller thread counts hurt decode. Batch2048/CPU44 improved review prefill to410.5tok/s but reduced decode to27.3tok/s. Batch2048/CPU42 failed GPU allocation. No overall performance winner accepted yet.

## Leading hypotheses to investigate after baseline

1. PLE sparse mmap page faults / over-reading on SATA; upstream PR28136 adds direct lazy-table reads. Measure bytes, faults and real varied prompts before choosing strategy.
2. CPU expert matrix multiplication, GPU expert placement and transfer overlap; prior35B custom kernels do not automatically support this quant/architecture.
3. Batch/microbatch, physical-core vs SMT scheduling, polling, CUDA graph/FlashAttention behavior.
4. Optional runtime n-gram and MTP speculation: acceptance/correctness are distinct from PLE and must be measured separately.
5. Newer upstream and compatible Flash-Next fixes in isolated source/build trees; never assume newer wins.

Final report and reproducibility materials will replace this provisional status with measured conclusions; separate P14 release/endurance work remains paused.

## Additional user authorization

Newer compatible Flash-Next model files may be downloaded and tested if disk space permits. No more aggressive/lower-quality quantization, pruning, or reduced model computation is allowed. Preserve all existing models. Alternate-model comparisons remain separate from same-file lossless execution experiments.

## Newer and original source provenance

Downloaded exact Unsloth original source archive92cedc8679d145902ead3f006258e8672eac11e6 and Sep11 newer source/bundle329b6160f513915f1c607dbfae3d5ce864a64a4f (b10909-mix-bea84f7). Recorded asset sizes/digests and isolated paths in download-manifest.json. Original source already supports Flash-Next recurrent rollback and grouped MTP RMSNorm; newer source changes history lookup, indexer execution and normalization paths. No claim that rollback is newly added relative to this bundled original. Current upstream master was independently pinned via API to97e4ca73582084f2751767f80c86237483ecc381; not yet built/tested.

A native CPU-only backend build and a thresholded parallel-PLE-gather variant were built and tested. They reuse the same original CUDA/model libraries in separate copied bundles, isolating CPU compilation and the gather scheduling change. No concurrent building and timed inference.

## Second screen and profiling checkpoint

- Newer Unsloth stock: review~299tok/s, decode~28.2tok/s; no meaningful speed gain. GPU peak~14469MiB, about284MiB lower than initial baseline.
- Newer `--load-mode none --lazy-mode on`: review386.9/431.3tok/s, prose TTFT0.81s instead of~1.21s, exact edit passed2/2. Generation unchanged~28-29tok/s. Load76.8s, and ~34GiB additional non-reclaimable RAM versus mmap. This is a prefill/latency tradeoff, not a loading victory.
- Original CPU backend rebuilt for Zen3 and parallel-PLE version: no meaningful warm gain. Three balanced independent OS-PLE-cold review starts per arm: both~286-287tok/s. Live mappings confirm intended experimental CPU libraries loaded. All tested exact edits passed.
- Native API function-call smoke passed all second-screen variants. Arithmetic answer323 correct but wrapped JSON fences in all builds, matching original format limitation.
- Separate original diagnostic combined CUPTI+perf (excluded from performance comparisons; measured overhead substantial). Zero dropped CUPTI records and zero perf lost samples. Prefill roughly161GiB H2D traffic for6250-token review,~10s measured GPU copy activity; many small IQ1_M dequantization/launch calls. Decode CPU samples emphasize MXFP4/IQ1_M/IQ2_S dot products and OpenMP waiting. See perf-original-report.txt and cupti-original-summary.json; phase boundaries have~0.5s telemetry resolution.

## Third screen in preparation

New isolated source copies from Sep11 exact source: source-new-register, source-new-pool, source-new-direct. Build all non-CUDA components locally; reuse exact newer CUDA backend because these patches do not change CUDA source/ABI. Controls test the local toolchain separately.

- Optional registration of mapped non-PLE host weights through existing CUDA register_host_buffer hook. Merge page ranges, preserve file-backed bytes, unregister before mappings are destroyed. PLE excluded. Environment gated; measure load overhead and transfer benefit.
- Port current expert-pool fork1582fccab9cfadb423e653de3c36516d02d72d63, with explicitly bounded experimental pool budget for preallocated hybrid KV. Default upstream estimate would charge recurrent layers and already allocated KV again. No routing changes. Cache-map correctness must be verified.
- Port PR28136 direct PLE reads to Qwen4exp only; restore unrelated Gemma4 and llama-bench hunks. Buffered pread, identical Q5_1 dequantization, no O_DIRECT claim. Keep worker creation as upstream for first measurement.
- Build no-OpenMP CPU library for the newer local control and compare native worker pool/polling to OpenMP passive waiting.

All original paths/models/configuration untouched. Further candidates and real AG1 validation remain outstanding; investigation active.

## Third screen complete / fourth screen active

- Read-only mapped CUDA registration failed on this driver (801, unsupported). Hardware query reports host registration supported but read-only registration unsupported. Third-screen labels containing `registered` mean an unsuccessful registration attempt with fallback, not pinned memory.
- Local rebuilt server control matches the newer release. Native non-OpenMP worker pool improves decode only about1%; disabling polling lowers CPU use to~700% but slows generation. OpenMP passive waiting and24 decode threads both regress.
- Direct PLE buffered reads showed no meaningful warm benefit. GPU expert pool32/CPU48 reduced decode to~18tok/s despite GPU execution; transfer traffic became the bottleneck. All exact edits and actual function-call smoke tests passed; the baseline JSON-fence limitation remained.
- Paired-row MXFP4 CPU dot kernel passed655,360 bit-for-bit checks and improved its microbenchmark~6%, but did not materially improve real-model decode. Patch retained as rejected evidence.
- A bounded24MiB pinned staging ring uses backend events to reuse host slots safely and transfers unchanged weight bytes. Pool64 plus staging reaches~25tok/s but is still slower than original. Larger pools/batches remain under test.
- Private copy-on-write mappings plus CUDA registration successfully pin non-PLE weights while keeping the original files opened read-only and unchanged. First model-scale test: load26.26s, review390.85/427.73tok/s, prose TTFT0.823/0.810s, exact-edit TTFT2.455/2.476s, generation~28-29tok/s, both edits exact. This retains pinned prefill speed with less loading cost than the76.8s allocation/copy approach, but cached original loads remain faster. RAM/page-cache/swap pressure must be assessed before recommendation. PLE remains excluded from pinning.

Optional n-gram/MTP speculation, full newer CUDA build, additional CPU scheduling, long-context validation and actual AG1 desktop comparisons remain outstanding. No overall winner or production change has been accepted.


## Expanded fork investigation and quality clarification

TheTom/llama-cpp-turboquant407f3237bfb3eeaff61546797de3d8c1a96be748 (Sep7) now contains qwen4exp model support and the hybrid CPU/GPU expert cache used in the earlier35B family of experiments. Source is being obtained and built separately; no additional model is being downloaded. Explicit Q8_0 K/V and disabled automatic/asymmetric Turbo cache selection preserve the requested cache precision. Initial comparison includes cache off at CPU42, cache off at CPU48, and bounded4480/6000MiB cache at CPU48. The installed older TurboQuant tree remains untouched. `/home` had288.05GiB free before this build.

A strict output-byte audit corrected the interpretation of the native source-edit screen: all55 checked responses matched after stripping outer whitespace but omitted the original trailing newline. These are content-correct, not literally byte-identical file results. See quality-adjudication.md and exact-edit-byte-audit.json. This behavior is shared by original and experimental configurations so far. Actual AG1 file writes will be checked byte-for-byte later.


## CPU scheduling validation and queued builds

The isolated flat single-token CPU scheduler passed63 matched output-file comparisons after restricting the specialization to preserve the stock IQ panel path for duplicate experts. The first broad guard produced small floating-point differences only in six duplicate-ID IQ cases; that failed attempt is retained in flat-fixture-mismatches.json and explained in flat-fixture-initial-failure.md. The corrected patch falls back for that shape and has not yet demonstrated a real-model speed gain.

Current build window compiles full sm89 CUDA backends for upstream97e4ca7, newer Unsloth329b6160f and TurboQuant407f323. No compiler runs concurrently with timed model inference. The post-build dispatcher tests the new hybrid-cache fork first, followed by flat CPU scheduling, graph scheduling, isolated ngram-mod arms, full-build comparisons and quantization-aware expert placement. Scripts/console logs document explicit stage dependencies; the initial fifth preparation stopped at the failed numerical check and resumed only after it passed.


## Full CUDA builds and fork checkpoint

Full sm89 builds of upstream97e4ca7, newer Unsloth329b6160f and TurboQuant407f323 completed. Only the stock TurboQuant real-model screen is currently running; the other builds remain queued. Disk reserve remains about286GiB; no extra model files were downloaded.

Fourth-screen results reinforce the tradeoffs: private-pinned batch2048/CPU44 reached warm review526.31tok/s but generation fell to roughly22–23tok/s. Bounded staging at batch4096/CPU48 reached458–481tok/s prefill with only~21tok/s normal decode. These are not accepted general improvements. Source-edit successes in older notes mean content equality after outer-whitespace stripping; the strict-byte correction above applies throughout.

The stock TurboQuant source does not mark Flash-Next PLE lazy and its PLE graph input does not implement graph reuse. An isolated single-translation-unit overlay adds separately enabled `AG1_FLASH_PLE_LAZY=1` and `AG1_FLASH_PLE_REUSE=1`; all other linked objects and CUDA kernels are the exact stock fork build. The patched library is in a separate bundle, and the source patch/build/link commands are archived. No speed claim for this overlay yet.

A stale `turbo-screen-done.json` from an earlier failed-build dispatch caused the first checkpoint controller to admit the overlay build and cache unit tests during `turbo-cache-off-cpu42`. The overlap was approximately16:30:35–16:30:42 local; that run ended16:31:30. The whole run is excluded from performance comparisons in invalid-runs.json and run-summary.json. Later stock arms started after the overlap. Raw results and the failed controller log remain preserved. The corrected checkpoint waits for the actual stock-screen PID to exit and requires a fresh completion marker; it pauses only the waiting parent dispatcher. It will repeat the stock control as `turbo-stock-cpu42-clean` and run clean unit tests before the overlay matrix. This is a correction to earlier blanket statements about no build overlap.

The global/production runtime and original model files remain untouched. No overall winner has been accepted.


## Next source experiments and serialized queue

The repaired PLE overlay matrix waits for the actual stock-fork screen process to exit, then repeats the excluded control and tests lazy-only, lazy+reuse, CPU48 control and hybrid-cache arms. The waiting parent dispatcher then resumes CPU-flat, graph controls, isolated original ngram-mod tests, full CUDA build comparisons, placement and improved GPU-pool scheduling. A further controller waits for that actual parent PID to exit and a fresh completion marker before any new work. It tests cache admission/copy scheduling/threads/batching (including a real capacity increase via smaller reserve), repaired-fork ngram2/4, existing Q8 MTP sidecar, and an isolated CUDA masked-warp experiment. No additional model is downloaded.

The proposed CUDA patch skips a warp's K/V work only when all32 rows are already masked with negative infinity, restricted to head256, one query, Q8_0 K/V and contexts at least4096. It neither changes attention membership nor reduces KV precision. `prepare-maskskip.py` and `test-maskskip.cpp` are prepared but not built/run yet. The fixture covers20 cases including sparse blocks, scattered masks, dense, empty and tail-only masks, compares exact GPU output bytes across two balanced process pairs and times kernels. Real-model and near83k-context comparisons are contingent on exact correctness and a meaningful microbenchmark gain.

`RUNS.md` is a generated audit catalog; `REPRODUCTION.md` explains how to adapt the protocol to another model. Both remain working evidence, not a completion claim. Actual AG1 validation, full-answer quality review and final preservation audit remain outstanding.


## Clean fork control and validation preparation

The clean stock CPU42 fork control completed: load157.10s, review warm321–323tok/s, edit24.59–25.42tok/s. The separately enabled lazy-PLE arm reduced observed load to9.24s and completed three edit outputs with matching content (the known trailing-newline limitation remains). Graph reuse and repaired cache arms are still running/queued; no overall win yet. Clean GPU cache unit tests returned exit0.

A source review identified a separate PLE-history rewind concern in the fork: retaining only two preceding tokens cannot reconstruct both predecessors after a rewind. See turbo-ple-rollback-review.md. Fixed-prefix append/rewind probability diagnostics and canonical teacher-forced comparisons are queued for original versus repaired fork before interpreting speculative speed. This concern is separate from the graph-reuse patch.

Private full-AG1 preparation and desktop scripts now exist under .local/ag1-flash-validation-20260914. They have only been parsed, not executed; capture/model hashing must wait until benchmark queues stop. They preserve all28 original shards in custody, use a private profile/capture/port/authority/workspace and the installed AG1 desktop on authenticated Xvfb without screenshots. Candidate selection, captures and actual runs remain outstanding.

A further optional copy-on-write prefault source overlay is prepared as prepare-cow-prefault.py, not built. It moves private page materialization before CUDA registration and parallelizes it across an explicit number of CPU workers. It writes identical bytes only to MAP_PRIVATE pages over O_RDONLY file descriptors. This targets the26s private-pinned load cost, not model mathematics; speed and memory-pressure effects remain unmeasured.


## PLE history repair prepared

The fork follow-up now includes original/fork append-and-rewind probability diagnostics, followed by a separate `bundle-turbo-history` build and matched real-model validation before speculation. `prepare-turbo-history.py` is prepared but not built. It retains live contiguous prefix token IDs (roughly360KB at90k context) instead of only the last two, copies only the needed tail into each hash calculation, preserves surviving history during prefix eviction, and bounds private state-blob restoration. It does not change tensor mathematics or model precision. It must pass real prefix-reuse/rewind, canonical teacher-forcing, edit and tool checks before recommendation. The current running graph/cache matrix still uses the earlier overlay and remains useful for isolating performance costs, not for closing correctness.


## Repaired-fork matrix completed; subsequent screen resumed

The checkpoint controller completed all lazy/reuse/cache arms and `original-after-turbo`, then resumed its waiting parent. The original control returned review~28.0, prose~29.0 and edit~28.9tok/s, consistent with the baseline. Repaired-cache review reached~31.6tok/s (default eligibility) and~33.5tok/s (including the existing IQ1_M experts), but edit fell to~24.9 and~22.7tok/s respectively. These are workload-specific tradeoffs, not a recommendation. The native rate numerator differs by one token across revisions; RUNS.md now includes a common normalized decode column, and new runner invocations record both common server-time and observed stream-time rates. The correction is under0.4% for these screens.

The corrected non-cache flat CPU scheduler is now in real-model testing, showing initial prose/edit~29.8tok/s. A separate cache-aware variant is prepared in `prepare-turbo-flat.py`: flatten only CPU miss tiles after the existing cache plan and publication barrier, retaining stock dot functions, row masks, GPU dispatch/collection and failure fallback. It will build and compare63 CPU output artifacts only after prior servers stop, then receive real-model/teacher-forced checks. It is not yet built or validated.

The serialized fork follow-up adds explicit CPU overlap0/4/8 and fusion-disabled controls because the automatic overlap calibration settles on the first workload and may be poorly matched to later editing. Periodic existing cache counters will aid per-workload diagnosis. `further-source-screen.py` is prepared but not launched; it covers parallel private-page prefault, full-expert prefill transfers and a larger bounded staging ring after the current queues end.


## N-gram thread checkpoint and adaptive work

The original ngram-mod sweep completed in both orders. All compared edit, prose, reasoning and repeated-rewrite outputs matched their no-speculation counterparts exactly. This preserves the baseline formatting failures too; it is not a claim that every output obeyed instructions. Edit acceleration was real but variable: depth16 reached41.2 then49.6tok/s. The repeated-rewrite fixture was slower (roughly19–21tok/s versus29) because only about half its proposals were accepted. This fixture changes a value in every repeated block; it is an adverse repeated-edit workload, not a pure copy upper bound. Additional verbatim source/repetition controls are prepared in extra_workload.py and will freeze separately without rewriting original prompts.

A bounded controller paused only the waiting dispatcher until its actual speculation child exited and produced a fresh marker, then compared12/24 batch threads in reverse order. Depth16 with12 physical threads reached53.16 and54.16tok/s, versus51.75 and51.52 with24threads in these two matched repetitions. All used identical prompts, quantization, placement and sampling; accepted draft counts and final outputs were unchanged. This remains an editing-specific improvement requiring broader validation. The controller has completed and resumed the original dispatcher.

Prepared, unbuilt common-library overlays now cover ngram recurrent snapshots and per-sequence proposal backoff after a low-acceptance32-token window. Backoff preserves target sampling/verification, continues updating the lookup table, and temporarily omits proposals rather than changing model computation. The intended test compares no speculation, fixed-depth speculation and adaptive speculation on both successful exact edits and the repeated-rewrite counterexample. The builders target the actual shared libllama-common layout and assert independent regular output libraries before linking.

## Current continuation checkpoint

The post-build screen completed, including all quantization-aware placement and efficient GPU-pool arms. These did not establish an across-workload winner. The post-screen controller is now running original/fork cached-prefix diagnostics before building retained PLE history and CPU miss scheduling overlays. A further-source controller waits for the actual current controller PID and a fresh completion marker; it queues recurrent-snapshot speculation, adaptive drafting, COW prefault, bulk prefill transfers and larger staging slots. A final waiting controller queues the separate higher-precision AtomicChat4.27 model download/test only after those timed runs exit cleanly. No heavy download or compiler overlaps timed inference.

The current primary model matches all28published AtomicChat shard hashes despite its local unsloth directory name. Remote headers for the4.27alternative show unchanged tensor shapes and higher-precision expert quantization; see alternative-model-assessment.md. Disk reserve was285GiB; the separate88.034GiB download enforces80GiBremaining. No additional model has been downloaded at this checkpoint.

REPORT.md now contains the preserved original configuration, baseline metrics and measurement definitions. It is explicitly a working report; final comparisons, AG1 validation and preservation audit remain pending. Teacher-force canonical suffixes now freeze the full baseline response tokenization so later64-position comparisons cannot silently reuse only a32-token prefix. The benchmark runner can validate alternate-model tokenization against all frozen prompts before timed requests.

### CPU miss overlay build recovery

The flat CPU overlay initially stopped before compilation because its build script assumed the newer ggml object-directory layout. The older fork uses `ggml/src/CMakeFiles/ggml-cpu.dir/ggml-cpu/ggml-cpu.c.o`. Both failed preparation directories were retained under `*-failed-command-layout`; no timed inference overlapped the recovery. The corrected isolated build passed63exact CPU fixtures (395,706float values). The original source file hash remained unchanged.

The prior chained controllers correctly aborted without fresh completion markers. Work now resumes under one sequential `continue-investigation.py` controller: remaining fork tuning, Q8 MTP and masked-warp tests; then the further source screen; then the separate higher-precision model download/test. Completed diagnostics and history benchmarks are reused, not rerun. Logs use `*-resume-console` / `*-continuation-console` names. This supersedes the previously described waiting-controller PIDs.

### Cache and load follow-up scope

The flat CPU miss schedule was rejected: normalized decode ratios versus its control were0.995review,0.960prose,0.950edit,0.971conversation. The larger usable cache improved prose to about34native tok/s, but edits remained about25tok/s. Reduced decode thread counts also regressed. A subsequent combined screen will test existing IQ1_M eligibility with the measured best cache configuration, partial permanent GPU offload plus smaller caches, and ngram4 with12/24batch threads.

Pinned-load telemetry revealed unequal disk work in earlier screens: newer-pinned read30.7GiB during its76.8s load, COW read0.091GiB during26.3s, and pool64-pinned read34.7GiB during97.2s. These observed durations cannot alone establish an intrinsic load-speed comparison. Upcoming prefault and original/CUDA-host load controls explicitly prewarm non-PLE weights outside the timer and retain actual load I/O telemetry. See pinned-load-io-comparison.json.


## User-requested pause for OS switch — 2026-09-14

All owned experiment controllers and the maskskip candidate server were stopped. Do not resume until the user requests it. Detailed root handoff: [HANDOFF-FLASH-NEXT-PERFORMANCE-20260914.md](../../../HANDOFF-FLASH-NEXT-PERFORMANCE-20260914.md). The maskskip candidate had startup artifacts but no completed result. MTP and maskskip fixtures/base-real are complete; further-source and alternative download have not started. All175 protected small files rehash unchanged;28 model shard sizes unchanged, full final model hash audit still due. Targeted resume script is prepared and unexecuted.


## Explicit resume after OS switch — 2026-09-14

The user explicitly requested reading the Flash-Next handoff and continuing. Fresh startup audit passed all 175 protected file hashes/symlink targets and all 28 primary shard sizes; 283.1 GiB disk free, about 59 GiB available RAM, and no competing inference/build process. Production still resolves to the preserved release. See `resume-machine-audit-20260914.json`. The targeted `resume-after-user-pause.py --resume-after-reboot` controller has started under active supervision, with fresh control/candidate run names. Final full model hashing remains deferred until timed inference finishes.


## Post-reboot masked-attention results

Both fresh 12-request control/candidate runs and six balanced 84,859-token runs completed. Every long-context response passed strict JSON retrieval. Two-run mean decode rates: original20.024, newer base20.620, masked candidate23.319 tok/s. Candidate versus matched newer base: about13.1% faster decode, but only0.9% lower fresh full-response latency because prefill remains about299seconds. Short review/prose/edit changes were around1% or less; cached-conversation decode regressed about3%. See `maskskip-resume-comparison.json`, `long-context-resume-summary.json`, and `long-context-resume-quality.json`.

Affected-path fixed-prefix diagnostics were added with base/candidate/base controls at6,250prompt tokens. Candidate greedy matches62/64; repeated control63/64. Mean coarsened top32-plus-residual KL0.00397candidate versus0.00287repeat control; real-model distributions are not bit-identical even between control repeats. These are diagnostic lower bounds, not full-vocabulary quality scores. The earlier prose/edit probes matched128/128reported distributions but did not activate the patch. See `maskskip-affected-review-distributions.json`.

The further-source queue is now running, beginning with Turbo combined cache/placement cases. No new production configuration has been selected. The private AG1 harness required correction: installed normal desktop lacks the DOM broker, so a separate current-source visual-test build is queued after benchmark work stops; see `resume-validation-notes.md`.


### Adaptive drafting repeated screen completed

All six arms produced the same output bytes for each of five workloads. Fixed16 source copying reached59.44/61.42tok/s versus26.96/25.65 without drafting; editing51.04/48.32 versus26.72/28.94. Repetitive rewrites slowed23.56/19.17 versus26.99/26.47. The1024-token adaptive cooldown recovered only part of that loss and prematurely disabled profitable repetitive copying. No automatic-policy recommendation yet. Mechanical content/fence evidence: `ngram-adaptive-quality.json`. COW prefault/load screening is active.

Prepared, not launched: `R/resume-targeted-followup.py` closes original1536 batch, same-order Turbo control,12-thread snapshot, and shorter adaptive cooldown gaps after the active controller exits. It does not restart or alter the active queue.


### Pinned-load prefault screen completed

Three balanced repetitions per worker count:0workers median20.6266s,6workers22.6314s,12workers21.1090s. Zero explicit prefault workers selected for later combinations. Each warm-file preparation is outside the load timer and recorded separately; typical subsequent preparation itself takes~40s. This is not a cold-load readiness figure. Evidence: `cow-prefault-load-comparison.json`.

All recurrent-snapshot enabled/disabled output pairs at depths4and8 were byte-identical, both cycles; enabled fresh/reuse diagnostics matched next content6/6 at each depth. See `ngram-rs-all-pairs.json`; this remains a limited next-token diagnostic, not full-state proof.

Private desktop preparation now creates separate original/candidate task and cancellation/recovery workspaces; scorer checks both kinds. These scripts remain prepared until timed model queues finish.


### Transfer comparisons completed

Whole-expert copying without pinning slowed matched review prefill286.15->252.03tok/s and301.27->258.00tok/s in reversed process orders. Reported fixed-prefix distributions matched128/128 across prose/edit probes; edit prefixes exercise the>=512token transfer branch. Decode variation also occurs in controls and must not be attributed to this prefill-only branch.

With COW pinning, whole-copy and selected-expert transfers were approximately tied; no substantial additive win. Preserve the simpler selected-expert transfer for candidate consideration. See `bulkcopy-pair0.json`, `bulkcopy-pair1.json`, `bulkcopy-quality.json`, `cow-bulkcopy-comparison.json`, `cow-bulkcopy-quality.json`. Unprepared repeated pinned loads were66–72s after previous pinned processes, versus20–23s with explicitly warmed files; cache state is part of readiness cost. Bounded staging8/64MiB screens remain before the alternative-model stage.

Final audit now also checks1144installed package files against `resume-installed-package-reference.json` (a packaged-release reference frozen during resume, separate from the original175file baseline). Full hashing has not yet run.


### Discovery closed after user feedback

User questioned the number of remaining tests and diminishing returns. Discovery is now closed: all34prepared targeted follow-up runs are disabled, and the optional88GiB higher-precision model download/test was deferred before starting. The old resume controller exited successfully; it must not be replayed. Evidence: `close-discovery-user-steering.json`, `alternative-model-test-deferred.json`.

One final candidate was assembled from already-screened private COW pinning and masked attention, with stock ngram16/twelve verification threads. Original primary weights,90kcontext,Q8K/V,batch1024andCPU42placement remain fixed. No adaptive policy, recurrent-snapshot extension, bulk copying, staging or Turbo bundle is included. ABI headers match and assembly hashes are in `closeout-candidate-assembly.json`; configuration is `R/closeout-candidate.json`.

Bounded closeout: `final-validation-suite.py closeout-candidate.json --focused-closeout` performs six full-answer processes (original/config-only/candidate, reversed) and two long-context processes. Actual AG1 uses two sessions with `desktop-run.py ARM --include-lifecycle`, followed by `score.py --combined-lifecycle`; cancellation/recovery shares each task session instead of additional fresh hosts. Then full preservation audit, archive and report. Do not add optimization branches. Private desktop build is active before timed validation.

## Focused closeout: native full-answer comparison finished (2026-09-14T23:36:43.259135+00:00)

All six full-answer process runs completed. Original/drafting-only/combined mean edit decode: 27.25/52.87/56.12 tok/s. Review prefill: 293.89/281.23/386.68 tok/s. Review decode is essentially unchanged; repetitive rewriting regresses. The report now contains the full table, startup/memory costs and manual quality findings. All cached-conversation answers hit the 1,800-token cap; shared exact-formatting failures remain failures. The final original long-context run passed strict factual JSON (TTFT 301.14 s, response 311.01 s); the combined long run is active. Discovery remains closed. After this last native run: two private AG1 sessions with lifecycle checks, preservation audit, reproduction archive and final report. No production cutover.

## Final closeout

Eight final native processes and two corrected actual AG1 sessions completed. One earlier desktop session failed before inference; retained evidence explains its canonical thinking-profile setup correction. All content/tool/recovery/cleanup checks passed in corrected sessions; both backend-retention checks failed because Stop terminated the process. The combined candidate accelerated ordinary app tasks but slowed initial readiness and recovery. The final preservation audit and 120-file reproduction archive passed. Retain the original default; optional configurations remain separately available.
