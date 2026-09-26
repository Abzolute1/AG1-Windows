# Flash-Next performance investigation — completed

**Production update, 15 September 2026:** the user subsequently authorized the Flash-Next switch. The optimized tune is now deployed; see the [deployment and verification record](implementation/evidence/flash-next-cutover-20260915/REPORT.md). The existing 35B-A3B profile, executable, library path and complete sealed launch-plan hash remain identical, using `/home/alex/Projects/llama-cpp-turboquant/build/bin/llama-server`. No rebuild or system installation was needed. The no-cutover statements below describe the earlier investigation checkpoint and are superseded by this deployment record.

**Completed 15 September 2026. No active experiment or remaining test queue.** Read the [final report](implementation/evidence/flash-next-performance-20260914/REPORT.md), [completion record](implementation/evidence/flash-next-performance-20260914/final-closeout.json), [run catalog](implementation/evidence/flash-next-performance-20260914/RUNS.md) and [reproduction guide](implementation/evidence/flash-next-performance-20260914/REPRODUCTION.md).

The final combined candidate achieved about 32% faster review prefill, 2.06× native source-edit decode, and 28% lower response latency at 84,859 prompt tokens. Actual AG1 chat/edit tasks were about 35% faster and web research about 16% faster in one completed comparison. Startup and post-Stop recovery were slower, with about 34 GiB private anonymous host memory; the full measured application sequence including startup/cancellation/recovery was slower. Keep the original default. The optional configurations and source bundle are retained separately for review.

Both completed AG1 sessions passed content, file, real-tool, new-chat reuse, recovery and cleanup checks. Both stopped/restarted their backend on cancellation, so strict backend-retention checks failed. One earlier session failed before inference because of a thinking-profile catalog mismatch; private canonical catalogs corrected the setup, and all failed evidence is retained. Native answer-quality defects and limits are documented, not converted into passing scores.

The final preservation audit passed for 175 protected entries, all 28 model shards, both MTP files, original source states and 1,144 installed package files. Production AG1, original runtimes/model and the earlier 35B tune remain preserved. No production cutover. The 34 additional screens and optional 88 GiB download were deferred before starting. Do not restart old controllers or the unrelated P14 backlog.

**Historical checkpoint below:** the remaining sections preserve the earlier OS-switch handoff. Their pause state and pending-work lists are superseded by the completed report above and are not executable instructions.

## 1. Immediate state and safe reboot

The owned experiment controllers and experimental llama-server were stopped before writing this file. The stopped controllers were `continue-investigation.py` and `resume-post-screen.py`; the active server was the separate `bundle-new-maskskip/llama-server`, listening on experimental port 18438. No experimental model, build or download was left running at the pause checkpoint. Unrelated user applications were not stopped. No user screen was captured.

Evidence: [user-pause-20260914.json](implementation/evidence/flash-next-performance-20260914/user-pause-20260914.json).

A fresh pause audit rehashed **175 protected runtime/configuration files: all 175 match their original hashes and symlink targets**. All **28 primary model shards exist with unchanged sizes**. To avoid delaying the requested reboot, the entire 79 GiB model was not rehashed again at pause; the initial full SHA-256 verification is retained and a final full hash audit remains required. See [preservation-at-user-pause.json](implementation/evidence/flash-next-performance-20260914/preservation-at-user-pause.json).

Disk space at pause: approximately **283.1 GiB available** on the experiment filesystem. The proposed 88 GiB higher-precision alternative model has **not been downloaded**. Its destination directory does not yet exist.

## 2. Mission and non-negotiable constraints

Improve real Qwen3.8 Flash-Next use on Ryzen 9 5900X, RTX 4080 16 GiB, 64 GiB DDR4-3600, Arch Linux. Priorities: prefill, decode, speculative generation, TTFT, full AG1 response latency, readiness/load time, CPU/GPU utilization, transfers, long context, exact editing and ordinary prose/reasoning. Do not substitute an isolated microbenchmark victory for application performance.

- Preserve original runtimes, source trees, binaries, symlinks, model shards and global configuration. Use separate source/build/bundle paths for every experiment.
- No lower-quality quantization, reduced experts, skipped layers or altered model mathematics. The user explicitly rejected “a more quantized, dumber version.”
- A separately downloaded higher-precision/newer compatible model is authorized if disk permits. It is a different-model/quantization experiment, not a lossless execution comparison.
- One GPU: **never overlap timed inference with another model, CUDA unit tests, compilation, model hashing/copying or heavy downloads**.
- No capturing the user's display. Actual AG1 validation must use the prepared private authenticated Xvfb/DOM setup.
- Checkpoints should be proportionate; do the deeper verification at the end. Keep meaningful failures and raw evidence.
- Do not resume the unrelated P14 endurance backlog. Do not change the preserved earlier Qwen3.6 35B-A3B tune.
- Do not spawn additional agents unless the user or applicable instructions explicitly request delegation. The old `source_review` agent is idle and unnecessary.

## 3. Where everything lives

| Purpose | Location |
|---|---|
| AG1 project / this handoff | `/home/alex/Projects/AG1` |
| All experimental source, builds, bundles, runners and configs (`R` below) | `/home/alex/Projects/flash-next-perf-20260914` |
| Evidence (`E` below) | `/home/alex/Projects/AG1/implementation/evidence/flash-next-performance-20260914` |
| Private AG1 validation scripts, prepared but not run | `/home/alex/Projects/AG1/.local/ag1-flash-validation-20260914` |
| Original protected server bundle | `/home/alex/Projects/llama-cpp-qwen4exp-mtp-b10715` |
| Original protected source | `/home/alex/Projects/llama-cpp-qwen4exp` |
| Original protected earlier 35B tune/source | `/home/alex/Projects/llama-cpp-turboquant` |

Start with [REPORT.md](implementation/evidence/flash-next-performance-20260914/REPORT.md), [RUNS.md](implementation/evidence/flash-next-performance-20260914/RUNS.md), [REPRODUCTION.md](implementation/evidence/flash-next-performance-20260914/REPRODUCTION.md), and [STATUS.md](implementation/evidence/flash-next-performance-20260914/STATUS.md). These are working documents; earlier statements that work is “active” mean unfinished, not permission to restart while the user is away playing.

At pause, the generated catalog contains **587 recorded real-model requests**, across **125 passed process runs, 1 failed, 1 instrumented, and 2 excluded runs with result files**. “Passed” means the process/runner finished; it does not certify all output quality checks. The interrupted maskskip candidate has only startup artifacts and no `result.json`, so it is separately recorded and not included in these totals. Teacher-forcing probes and CUDA microbenchmark iterations are additional diagnostics, not included in that request count.

Per-run directories contain `launch.json`, `server.log`, request/results, telemetry and loaded-library evidence. Their exact recorded argv/environment take precedence over abbreviated tables here.

## 4. Original runtime and AG1 integration

Original server: `/home/alex/Projects/llama-cpp-qwen4exp-mtp-b10715/llama-server`.

Version **0.3.0-dev, build 10715**, commit **92cedc8679d145902ead3f006258e8672eac11e6**, Unsloth tag `b10715-mix-86bd2d3`. Prebuilt with GNU 11.4 / CUDA 13.3, shared dynamically selected CPU backends, CUDA architectures including sm86/89/90/100/103/120. The original selected CPU backend is haswell; original OpenMP affinity is effective.

AG1 profile: `/home/alex/.ag1/agent/llamacpp-model-profiles.json`, model ID `Qwen3.8-Flash-Next-AD-3.84bpw-IQ4_XS-M64`.

Sealed runtime configuration: `/home/alex/.local/state/ag1/desktop/configuration/runtime-configuration-vision-20260909.json`; host configuration: `/home/alex/.local/state/ag1/desktop/native-host.json`. Installed release remains `/opt/ag1/current` → release `ff218780597542c58cec2e5c1ba4a025`.

Exact captured command/environment: [original-launch.json](implementation/evidence/flash-next-performance-20260914/original-launch.json). Original arguments:

```text
--model /home/alex/.lmstudio/models/unsloth/Qwen3.8-Flash-Next-AD-3.84bpw-IQ4_XS-M64-GGUF/Qwen3.8-Flash-Next-AD-3.84bpw-IQ4_XS-M64-00001-of-00028.gguf
--alias Qwen3.8-Flash-Next-AD-3.84bpw-IQ4_XS-M64
--host 127.0.0.1 --port 18080
--flash-attn on --ctx-size 90000
--cache-type-k q8_0 --cache-type-v q8_0
--no-webui --jinja --parallel 1 --fit-target 1024
--batch-size 1024 --ubatch-size 1024
--threads 12 --threads-batch 24
--cpu-range 0-11 --cpu-strict 1
--cpu-range-batch 0-23 --cpu-strict-batch 1
--poll 100 --slot-prompt-similarity 0
--no-mmproj --fit off --n-gpu-layers auto
--kv-unified --n-cpu-moe 42 --metrics --verbosity 2
```

Effective allocated context is 90112. The profile's nominal microbatch 512 is overridden by its 90000-context batch tier to 1024/1024. Original extraArgs are only `['--metrics','--verbosity','2']`. There is no original runtime speculation enabled. CUDA_PATH is `/opt/cuda`; bundle LD_LIBRARY_PATH is used; no original OMP/GGML overrides.

Hardware observations: NVIDIA driver 610.57.04, NVCC 13.3, installed GCC 15. Physical CPU cores 0–11, SMT siblings 12–23; each CCD has six physical cores. Storage hosting the model is a **Samsung 870 EVO SATA SSD**, not NVMe. Approximately 62 GiB physical RAM and 4 GiB zram. Desktop/Discord consume roughly 1.1–1.2 GiB VRAM, varying with user activity. Leave unrelated applications alone and record their effect.

## 5. Exact model and the crucial n-gram distinction

The existing primary model is **28 shards, 79.098 GiB**. All initial shard hashes matched the sealed AG1 model manifest. Stored hashes also match AtomicChat's published 3.84 bpw model byte-for-byte, despite the local directory being under `unsloth`.

The filename does **not** accurately summarize individual tensor types. Actual layout: 1224 tensors, with Q5_1 PLE (~35.763 GiB), MXFP4 down experts (~19.922 GiB), IQ1_M gate/up experts (~12.305 GiB), IQ2_S gate/up experts (~6.006 GiB), Q8_0 (~4.812 GiB), and smaller F32/BF16/F16 tensors. There are **no IQ4_XS tensors** in this file. The middle 36 layers already use IQ1_M gate/up experts; the first/last six use IQ2_S. No experiment re-quantized them.

Architecture: qwen4exp, 48 blocks, embedding 2560, 512 experts with 10 active; hybrid recurrent/GDN and full attention every fourth block. Full attention head dimension 256, 24 query heads / 2 KV heads. QSA selects 2048 token positions (selected width 2051), not 8192. Requested context stays 90000 throughout primary comparisons.

**Learned PLE n-grams and optional runtime speculative n-grams are separate systems.**

- PLE is a learned bigram/trigram embedding table: 320001536 rows, Q5_1, 120 bytes per row, 16 lookups/token → about 1920 raw table bytes/token. It is mainly demand-paged from RAM/storage, with small lookup outputs entering model computation. There is no draft acceptance rate for PLE.
- Runtime `ngram-mod` proposes token sequences from repeated context and the unchanged target model verifies them. Proposed/accepted counts belong here.
- The existing Q8_0 MTP draft head is a third, separate mechanism. It was tested without changing target weights. Its path and hashes are in `mtp-before.json`.

The user's IQ1_M question was answered: changing cache eligibility to include existing IQ1_M expert tensors moves the same stored weights between RAM/GPU; it does not further quantize them. Do not introduce IQ1_M weights into a model that did not already use them and describe that as lossless.

## 6. Baseline and measurement discipline

`R/bench.py` is the current runner. **Never run `R/prepare_runner.py`: it overwrites the improved runner.** Likewise, do not rerun `inspect_original.py` casually: it performs heavy hashing and can overwrite historical inventories. Use the existing manifests.

Frozen prompts are in `E/prompts.json`. Temperature 0, top-k 1, top-p 1, min-p 0, repeat penalty 1, seed 777; fresh workloads use cache_prompt=false and slot 0. Cached conversation is explicitly warmed separately. Experimental server port is 18438. Telemetry includes CPU/RSS/HWM/faults/disk I/O/swap/frequency/temperature/GPU utilization/power/clocks/PCIe; recent runs additionally record process-specific NVML VRAM.

Baseline `baseline-01`: three repetitions per class, 24 requests:

| Workload | Prompt tokens | Prefill tok/s | Decode tok/s | TTFT seconds |
|---|---:|---:|---:|---:|
| Code review | 6250 | 302.03 | 27.99 | 20.696 |
| Exact edit | 1006 | 296.56 | 29.02 | 3.393 |
| Prose | 74 | 61.12 | 29.19 | 1.211 |
| Tool-shaped JSON | 70 | 58.28 | 28.21 | 1.202 |
| Cached conversation | 65 new / 4243 total | 66.56 new | 28.36 | 0.978 |
| Repetitive rewrite | 500 | 193.23 | 29.09 | 2.588 |
| Reasoning | 110 | 76.29 | 29.07 | 1.443 |
| Fresh long input | 21308 | 305.46 | 25.97 | 69.762 |

Observed initial load 85.443 seconds; subsequent warm loads typically 3.4–5.4 seconds. This is not a standardized cold-boot comparison. Baseline peak total GPU memory ~14753 MiB, RSS ~36–38 GiB, HWM ~41.8 GiB. Review prefill GPU utilization ~65%, CPU ~100%, PCIe receive ~8 GiB/s. Decode GPU ~42.5%, CPU ~1193%, PCIe receive ~154 MiB/s. CPU percentages are process aggregate, so 1200% means about twelve cores.

**Timing correction:** original decode rate counts n_pred−1 while the older Turbo fork counts n_pred. RUNS.md reports both native and common `(n_pred−1)*1000/predicted_ms`; compare the common column. Difference is ~0.39% at 256 tokens and ~0.11% at 938, not enough to explain major gains.

**Load-cache confound:** CUDA-host pinned load 76.832 s read 30.715 GiB from disk, whereas COW pinned load 26.260 s read only 0.091 GiB. Do not claim that whole difference is an intrinsic source optimization. Controlled prewarm/load tests are prepared. `prewarm_weights=true` reads non-PLE primary shards before timing and separately records preparation time; it is a warm-file-cache test, not a cold load. Allocating pinned memory may itself evict source pages.

## 7. What has been learned and measured

These are screens, not final production recommendations. Complete per-run numbers and resources are in RUNS.md.

### Bottlenecks

CUPTI/perf on the original found approximately 161 GiB H2D traffic for a 6250-token review prefill, 10.102 s summed GPU copy time and 13.338 s CPU cudaMemcpyAsync API time. CPU expert decode and memory bandwidth dominate normal generation. Instrumented timings overlap and cannot simply be added; instrumentation also slowed decode to ~20–21 tok/s and that run is excluded from performance comparisons.

There were ~85k IQ1_M dequant launches, but only ~0.506 s summed GPU dequant time. That makes transfer/scheduling a stronger immediate target than blindly writing a new quantized kernel. CPU sampling showed OpenMP waiting plus MXFP4/IQ1_M dot products as major costs.

### Most useful candidates so far

| Candidate | Measured benefit | Important limitation |
|---|---|---|
| Original, batch/microbatch 2048, CPU experts 44 | Review prefill 410.51 tok/s; TTFT 15.227 s | Decode 27.28 vs 27.99 review; prose/edit also slightly slower; original CPU42 at 2048 OOM |
| Newer isolated private COW pinned experts | Review prefill median 409.29 (warm repetition 427.73); prose TTFT ~0.817 s; edit TTFT ~2.465 s; decode near original | Adds ~34 GiB private anonymous RAM; warm load ~26 s; requires controlled repeat/load tests |
| Modified Turbo cache, reserve 1024 | Prose ~33.72 common tok/s, review decode ~32.41 | Edit only ~24.82, below original ~29 |
| Modified Turbo, batch 1536 | Review prefill median 371.02, prose native 33.43 (~33.30 common), review decode native 31.63 | Edit native 24.83; TTFT prose ~1.31 s; not all-metric winner |
| Modified Turbo, batch 2048/cache4480 | Review prefill median 441.88; TTFT 14.150 s; prose native 31.90 | Edit native 24.77; cache/headroom tradeoffs |
| Original ngram-mod depth16, 12 verification threads | Exact-edit decode 53.159 / 54.162, median **53.661**, vs original ~29; same 938 output tokens | Ordinary prose receives little help; repetitive rewrite can fall to ~19–21; must not enable aggressively for everything |
| Masked-warp attention kernel | 83,200-position block-sparse synthetic case **3.443×**, exact outputs across 20 fixtures | Real-model candidate interrupted before completed requests; **not validated** |

The original ngram16/12-thread exact-edit response took ~21.0 seconds versus baseline ~35.7 seconds including prompt processing. Draft counts 848 proposed / 839 accepted. Twelve verification threads used ~1118% CPU versus ~2216% with 24; the latter's repeated median was ~51.63 tok/s. Results still need the final balanced full-workload validation.

### Meaningful ideas that failed or had tradeoffs

- Fewer decode threads, passive OpenMP, poll=0: generally slower; reduced CPU consumption alone was not an overall win.
- CUDA graphs off: decode dropped to ~23–25 tok/s. Keep graphs.
- Newer Unsloth and current upstream full Ada builds: broadly ~297–299 prefill / ~27–28 decode; no automatic newer-is-faster result.
- CPU flat work scheduling: newer build small ~2–3% screen gain; Turbo variant failed real inference despite 395706 exact fixture floats and was rejected (prose ~4%, edit ~5% slower).
- Paired MXFP4 CPU rows: ~6% micro gain, no useful real-model gain.
- Direct PLE pread/parallel reads: no meaningful improvement.
- Read-only host registration: driver returned error 801. Private writable COW registration worked without modifying GGUF contents.
- Fixed GPU expert pools: many configurations slower, especially decode ~18–26. An efficient 80-slot pool reached prose ~30 but not across all tasks.
- Bounded pinned staging: some prefill benefits at larger batch, often worse decode. 4096 batch/CPU48 gave ~469 prefill but ~21 decode.
- COW plus 2048 batch/CPU44 reached warm prefill 526.31, but decode fell to ~22–26. Reject as a universal tune.
- Expert placement changes: at most small gains, often less VRAM headroom. Not enough to claim a victory.
- Turbo cache async fill, explicit overlap0/4/8, lower churn, fusion off and fewer threads: no decisive all-workload win. Low churn helped edit slightly but remained below original.
- Turbo ngram4: edit ~28.9, far below original ngram16 ~54; repetitive rewrite ~20.
- Existing **Q8 MTP**, CPU46 matched control: control prose 27.20/edit27.15/reasoning27.21. Max2: prose26.71/edit32.91/reasoning25.86. Max4: prose22.15/edit37.09/reasoning24.70. MTP helps edit less than original ngram16 and hurts prose; not the leading candidate. Peak total VRAM ~14588/14820 MiB for max2/max4.

## 8. Fork, source changes and correctness caveats

Turbo source: [TheTom/llama-cpp-turboquant](https://github.com/TheTom/llama-cpp-turboquant), branch feature/turboquant-kv-cache, commit **407f3237bfb3eeaff61546797de3d8c1a96be748**. Local `R/turbo-latest-source/TheTom-llama-cpp-turboquant-407f323`, build `R/build-turbo-latest`, full sm89/GCC15/NVCC13.3.

Newer Unsloth: **329b6160f513915f1c607dbfae3d5ce864a64a4f**, tag b10909-mix-bea84f7; source `R/unsloth-new-source/llama.cpp-b10909-mix-bea84f7`, prebuilt `R/unsloth-new-bundle`, local build `R/build-new-ada`.

Upstream: **97e4ca73582084f2751767f80c86237483ecc381**, source `R/upstream-source/ggml-org-llama.cpp-97e4ca7`, build `R/build-upstream-ada`. Source SHA is authoritative when archive builds report unknown version.

Important separate overlays/bundles:

| Experiment | Script / isolated bundle | Source / evidence |
|---|---|---|
| Turbo lazy PLE + graph reuse | prepare-turbo-ple.py / bundle-turbo-ple | qwen4exp.cpp; turbo-ple-lazy-reuse.patch |
| Turbo retained PLE history | prepare-turbo-history.py / bundle-turbo-history | qwen4exp.cpp + llama-memory-hybrid-idx.cpp; turbo-ple-retained-history.patch |
| Turbo flat CPU misses, rejected | prepare-turbo-flat.py / bundle-turbo-flat | ggml-cpu.c; turbo-flat-cpu-misses.patch |
| Private COW pinning | port-cow.py / bundle-new-cow | private-cow-registration.patch |
| Bounded staging | port-stage.py / bundle-new-stage | bounded-host-stage.patch |
| Efficient GPU pool updates | bundle-new-pool-efficient | pool-efficient-updates.patch |
| Masked-warp attention, built | prepare-maskskip.py / bundle-new-maskskip | CUDA Q8 K/V attention D256 vector path; cuda-masked-warp-skip.patch |
| Parallel COW prefault, not built/tested | prepare-cow-prefault.py | cow-parallel-prefault.patch generated by builder |
| Whole expert prefill transfers, not built/tested | prepare-bulkcopy.py | ggml-backend.cpp; prefill-whole-expert-copy.patch |
| Ngram recurrent snapshots, not built/tested | prepare-ngram-rs.py | common.cpp / shared llama-common overlay |
| Adaptive ngram backoff, not built/tested | prepare-ngram-adaptive.py | speculative.cpp; ngram-adaptive-backoff.patch |
| Turbo private expert pinning, not built/tested | prepare-turbo-cow.py | llama-model.cpp + llama-mmap.cpp; turbo-private-expert-pinning.patch |

Builders preserve base source and redirect changed translation-unit objects/dependency outputs and linked libraries into new overlays. Exact build commands/manifests are in E. Do not mix libraries across different source/ABI families. In particular, the pool experiments expand context parameters and are not ABI-compatible with non-pool libllama.

Turbo default cache minimum expert size 512 KiB excludes existing ~350 KiB IQ1_M tensors. Setting minimum 256 changes eligibility only. Requested 6000 MiB cache may actually allocate ~4.2 GiB because of the reserve. Inspect logs for actual capacity. Cache logical CPU/GPU masks are uint64, and 10 experts/token means large speculative verification batches exceed that path: ngram4 fits (~50 logical rows), ngram16 uses the stock bypass. **Do not simply raise the row limit past 64.** Explicit CPU overlap overrides apply to all-hit nodes, not partial-miss nodes. There is substantial expert-cache churn; hit rate is not speculative acceptance rate.

The PLE lazy/reuse patch fixed the fork's eager ~79 GiB load behavior and inability to reuse graphs. Stock fork load was ~152–160 s; lazy/reuse warm loads ~3 s, with substantial decode recovery. These are not proof that the modified fork beats the original in load time.

Retained PLE history is defensive for a direct rollback counterexample. Current server diagnostics restore a checkpoint then replay, so they **did not demonstrate present AG1 PLE corruption**. Rewind1 restored checkpoint897 then replayed3; rewind4/32 recomputed the 900-token fixture. Do not present the history patch as a proven repair of user-visible corruption or a measured speed gain. See turbo-ple-rollback-review.md.

Teacher-forcing compared 32 positions each in prose/edit. Fork greedy matches were 30/32 prose, 32/32 edit; small numerical distribution differences exist. Reported KL is a **coarsened top32-plus-residual lower bound**, not full KL/perplexity or proof of unchanged intelligence. See teacher-force-comparison.json and quality-adjudication.md.

Exact text-edit fixtures commonly omit the final newline even in the original. Historical “quality passed” ignored outer whitespace. New fields distinguish content and exact bytes. Repetitive rewrite is fenced by the baseline and fork alike; byte-identical baseline failures must not be called new corruption. Tool-shaped text JSON fixture is ambiguous; actual forced function-call smoke tests pass. Short 256-token screens do not validate 500-word instructions. Full-answer factuality, formatting, instruction adherence and actual written-file bytes remain due.

## 9. Exact stopping point

Completed before pause:

1. Initial configuration/source/pool/placement investigations and baseline.
2. 32 original ngram screens; 12 original ngram verification-thread screens.
3. Turbo build, lazy/reuse/history, diagnostics, rejected flat CPU variant, all Turbo tuning followups (`turbo-followup-done.json` exists).
4. Matched CPU46 MTP control and Q8 max2/max4 runs.
5. Maskskip build and four balanced CUDA fixture processes: **all 20 fixture outputs exactly equal**, testing 2048/4096/21504/83200 lengths and block/scattered/dense/empty/tail masks.
6. `maskskip-base-real`: three repetitions each review/prose/edit/conversation, API smoke and teacher-force diagnostics completed.

Interrupted:

- `E/runs/maskskip-candidate-real/` contains startup artifacts only (launch, log, props, mappings, smaps, pid). It has no completed result.json. It must not be treated as a failed kernel or a completed benchmark. Keep it as pause evidence and use a fresh run name.
- `post-screen-investigation-done.json` does not exist.
- Near83k real-model runs have **not** started.
- `further-source-screen.py` has **not** started.
- `alternative-model-test.py` has **not** started; no alternative download.
- Final suite and actual private AG1 tests have **not** run.

Masked-warp latest micro result: at 83,200 block-sparse positions base0.489905 ms vs candidate0.142305 ms (3.443×); scattered1.631×; dense1.007×. At 21,504 block-sparse1.346×; some short/dense cases regress ~1–3%. These are individual attention-kernel timings, **not model tok/s gains**. The optimization skips only warps whose existing mask is entirely negative infinity; it does not change selected attention membership.

## 10. How to resume after the user returns

First read this file, relevant AGENTS.md instructions, E/REPRODUCTION.md and the fresh machine state. Verify free disk/RAM/VRAM, no competing inference/build processes, original paths and driver state. Reboot changes file-cache/thermal state; run balanced controls again rather than comparing only against a pre-reboot sample.

**Do not rerun `continue-investigation.py`, `resume-post-screen.py`, `post-screen-investigation.py`, or earlier whole controllers unchanged.** They include already-completed builds/runs, use exclusive output directories, and will collide with retained evidence. Earlier stale completion-marker scheduling caused one excluded overlap. Wait for actual process completion, not merely an old marker.

A dedicated **prepared, syntax-checked, not executed** continuation is now at:

`/home/alex/Projects/flash-next-perf-20260914/resume-after-user-pause.py`

Only after the user explicitly resumes, run:

```bash
cd /home/alex/Projects/flash-next-perf-20260914
python -u resume-after-user-pause.py --resume-after-reboot > /home/alex/Projects/AG1/implementation/evidence/flash-next-performance-20260914/resume-after-user-pause-console.log 2>&1
```

It repeats both maskskip base and candidate under fresh names after reboot, runs two balanced original/base/candidate near83k cycles, then sequentially invokes the as-yet-unstarted further-source and alternative-model stages. It refuses to start without the explicit flag or if a prior resume-start marker exists. It is not a service, cron job or autostart task. If interrupted again, inspect completed evidence and construct a targeted continuation; do not erase the marker/results to force a replay.

The continuation runs screens only. **Final validation, actual AG1, final audit and report still require active engineering after it finishes.** Read generated failures and adapt; no controller guarantees all experimental builders succeed.

### Further-source-screen queue, in order

1. Combine promising Turbo batch/cache options, including smaller cache with partial permanent GPU offload at CPU42/44/46, eligibility256, then ngram4/16 with verification-thread variants. Initial control selection prioritizes prefill among prose candidates within 3% of the fastest.
2. Original ngram16: six verification threads on one CCD vs twelve across both, decode remains twelve. Test edit/prose/repetitive in balanced order.
3. Newer ngram recurrent snapshot allocation, depths4/8 enabled/disabled, state checks. This may trade checkpoint overhead for VRAM/state-copy overhead.
4. Adaptive ngram backoff: after32 proposals with acceptance below0.85, pause drafting1024 tokens while maintaining context learning. Compare none/fixed16/adaptive16 on edit, repetitive and exact-copy fixtures. Target verification remains unchanged.
5. Controlled private-COW prefault/load tests with0/6/12 workers and warm-weight preparation; compare original and CUDA-host allocation under the same starting condition.
6. Combine Turbo cache with private expert pinning; same primary weights, merged mapped expert ranges, private writable pages, optional parallel prefault, unregister on destruction. Test off/on and optional ngram16. This is a plausible prefill+decode combination but still unbuilt/unproven.
7. Whole-expert prefill copy vs baseline, then combined with COW. For sufficiently large batches, transfer unchanged complete expert tensors instead of serial ID-readback/ranged copies; routing remains unchanged, but skewed workloads could waste bandwidth.
8. Existing bounded staging slots8 vs64 MiB (three slots), repeated real inference.

Flags caveat: old `AG1_FLASH_REGISTER_COW` checks **environment presence**, so setting it to `'0'` still enables it. For controls, omit it entirely. The newer Turbo-specific `AG1_FLASH_TURBO_COW` uses strict value `'1'`. Keep this distinction.

Potential followup only if evidence justifies it: Turbo fill worker copies CPU expert data into pinned staging even if a later COW experiment has pinned that source. A direct registered-source async copy might remove a redundant memcpy, but registration/lifetime/locking must be proven. No such patch has been implemented. Do not add complexity without measurements.

## 11. Authorized higher-precision model test

Source: [AtomicChat/Qwen3.8-Flash-Next-GGUF](https://huggingface.co/AtomicChat/Qwen3.8-Flash-Next-GGUF), pinned revision **142262902a46f7daed19c79d0771534c8106ad59**.

Candidate: `Qwen3.8-Flash-Next-AD-4.27bpw-Q4_K_M-M64`, 33 shards, **88.033634 GiB actual catalog size**. Remote metadata/header inspection is complete, not the download. Same1224 tensor names/shapes. Changes:68 IQ1_M→IQ2_S,4 IQ1_M→IQ3_S,20 IQ2_S→IQ3_S,48 MXFP4→IQ4_NL. PLE stays Q5_1. Publisher claims lower divergence; independently validate quality rather than treating that claim as proof.

`R/alternative-model-test.py` downloads into `R/models/AtomicChat-Flash-Next-4.27bpw`, retains an80 GiB free-space reserve, supports validated partial resume, checks SHA-256 per shard, and never overwrites the original. It then runs two balanced original/alternative cycles at matched CPU44, same90k/Q8/b1024/u1024 and frozen prompts, with tokenizer compatibility checks. Different quantized kernels might help or larger weights might hurt; no speed promise.

Evidence: alternative-model-assessment.md, alternative-427-assessment.json, alternative-427-remote-layout.json, atomic-model-catalog.json. Do not download an inferior quant to fit more in VRAM. No5.00bpw download is currently planned.

## 12. Required final validation and delivery

`R/final-validation-suite.py candidate-config.json` is prepared, not run. It compares original, original configuration-only ngram16/12 verification threads, and the selected primary-model candidate in three balanced orders, full-answer budget1800. Then it always runs original/candidate near83k context in balanced order. Do not skip the long-context proof even if a microbenchmark disappoints.

Manually adjudicate complete prose/reasoning, review factuality, actual tool calls and structured outputs, long-context retrieval, exact edits including trailing newline, output corruption and stop/recovery behavior. Record cache acceptance, resource costs and variance by workload, not one blended average. Compare winners against the simple configuration-only original, not just untuned baseline.

Actual AG1 scripts in `.local/ag1-flash-validation-20260914`:

- `prepare.py`: isolated original/candidate profiles/configuration/capture; hashes required material, private port18436, no production writes. Primary-model configs only currently; alternate model or remove-flags need explicit adaptation.
- `build-desktop.py`: build current desktop source with the `visual-test` DOM broker after timed queues stop, into a separate private executable. The installed desktop has no broker; it remains unchanged.
- `prepare-desktop.py`: private instrumented desktop, installed process host, current renderer, private authority, and separate task/lifecycle workspaces for each arm.
- `desktop-run.py`: real Tauri/WebKit from current source under authenticated private Xvfb/software GL, no screenshots. Test manual Load with unsent draft, new-chat model retention, simple chat, strict JSON source edit, search/fetch/citation, stop/recovery and clean quit. Assert actual backend argv, mappings and PID. Use `original`/`candidate` for task runs and `--label original-lifecycle --lifecycle` / `--label candidate-lifecycle --lifecycle` for separate lifecycle runs.
- `score.py`: native journal/tools/answers, strict written-file verification, and both lifecycle sequences. Review before execution. These diagnostic integration sequences do not statistically establish end-to-end speed.

AG1 readiness must include its own material verification: `MaterialVerifier` caches verified path/dev/inode/size/mtime/ctime/digest in memory, but a fresh host hashes all required materials including28 shards. Thus a3-second warm llama load does not mean3-second first AG1 model readiness. Measure material verification, server load and warm reuse separately. Do not weaken integrity checks to improve a number.

Before final delivery: rehash all protected files, all28 primary shards and MTP files outside timed inference; verify original source/git states and production paths. Archive final reproducible scripts, exact manifests and patches. Finish REPORT.md with every meaningful experiment, counts, medians/ranges, before/after prefill/decode/TTFT/load/readiness/resources, quality and stability findings, recommended build/configuration, workload tradeoffs and remaining opportunities. Keep the original if it remains preferable. No “true victory” claim until supported; no production cutover has been authorized by a completed validation yet.

## 13. Evidence pitfalls and resume checklist

- `invalid-runs.json`: initial Turbo CPU42 stock run overlapped a stale-marker-triggered build/GPU test; use clean rerun. `new-cpu-flat` interrupted before measurements; use verified replacement. Maskskip candidate interrupted by user pause; retain and rerun under fresh name.
- Early failed Turbo flat builder assumed the newer object's path; fixed old fork path is `ggml/src/CMakeFiles/ggml-cpu.dir/ggml-cpu/ggml-cpu.c.o`. Failed preparation directories retained.
- Newer329 has CPU IQ panel paths; original10715 does **not**. Do not explain original ngram superiority by a nonexistent original panel path.
- Do not rerun `write-report-foundation.py`; it asserts REPORT.md absent and was only for initial creation.
- Regenerate catalog with `python R/write-run-catalog.py`, summaries with `python R/summarize.py`; these only summarize evidence.
- Existing run/build names are not reusable. Keep raw logs and use new names for new conditions, especially after reboot.
- Neither lower runtime memory nor a faster isolated kernel automatically establishes better quality, long context or full AG1 behavior.

**The explicit resume and fresh machine check have now happened. Continue the active targeted controller; do not launch it again. After it exits, complete the comparison gaps in `resume-validation-notes.md`, select and validate a candidate, run the private AG1 sequences, and finish the preservation audit/report.**

A pause snapshot of 70 reproduction source/scripts is retained in `E/reproduction-scripts-at-user-pause.tar.gz`, with file hashes in `E/reproduction-scripts-at-user-pause.json`. This contains scripts, not model weights or full build trees; keep R and its existing build objects for overlay reproduction.
