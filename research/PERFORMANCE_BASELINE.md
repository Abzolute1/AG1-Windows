# Performance baseline and measurement gates

Measured on 2026-09-04, Arch Linux x86_64, Linux 7.1.8, Ryzen 9 5900X (12 cores/24 threads), approximately 62.7 GiB RAM, Node 26.7.0, npm 12.0.2. Source revision and build procedure are pinned in README. No model weights were loaded, no inference service was disturbed and no paid/network model requests were made. Shared-machine load, CPU frequency and filesystem cache were not controlled; these results are engineering baselines, not product claims.

## Isolation and methodology

The AG1 reference was produced by `git -C /home/alex/Projects/Cruzible archive HEAD` extracted into `research/scratch/ag1-reference`. Its dependencies were copied with `cp -a --reflink=auto`, not symlinked back to Cruzible. TUI, AI, agent and coding-agent were rebuilt from the pinned source in that copy. `AG1_CODING_AGENT_DIR` was redirected for CLI/tests. Test caches and generated fixture projects therefore live outside the original repository.

`evidence/measure-ag1.mjs` calls rebuilt production modules, creates its own Git project with 120 TypeScript files and uses fresh artifact/session directories. Samples are sequential; most operations use three warmups and 30 samples, scheduler 100 samples, large compiler/event cases 10. It records every sample; p95 is nearest-rank, p50 the upper middle sorted sample. No percentile confidence intervals are claimed. Filesystem cache is warm; repeated artifact writes reuse blob content but create records/index work. Source retrieval and catalog sizes grow during the run. These are explicitly not stationary real-project benchmarks.

Reproduce from the workspace root after rebuilding the copy:

```sh
node research/evidence/measure-ag1.mjs
```

The script also runs non-destructive defect probes against its own fixtures. It is a research harness, not new production code. Its two-second idle tail describes the loaded benchmark process after work, not the full AG1 interactive application.

## Measured AG1 results

| Operation (synthetic workload) | Samples | p50 ms | p95 ms |
|---|---:|---:|---:|
| `artifact_put_988000_bytes_deduplicated_blob_new_record` | 30 | 20.9524 | 21.8689 |
| `artifact_get` | 30 | 0.0094 | 0.0182 |
| `artifact_read_8192_bytes` | 30 | 0.0400 | 0.0669 |
| `artifact_search_marker` | 30 | 17.6843 | 19.2429 |
| `session_append_custom_async` | 30 | 0.2034 | 0.2716 |
| `session_reopen` | 30 | 0.1228 | 0.2104 |
| `workspace_index_uncached_120_files` | 30 | 42.0994 | 42.8661 |
| `source_context_exact_symbol` | 30 | 18.9131 | 20.1514 |
| `source_revision_hash` | 30 | 0.0610 | 0.0763 |
| `source_revision_warm_validation` | 30 | 0.0393 | 0.0494 |
| `git_status` | 30 | 3.3154 | 3.4940 |
| `git_diff` | 30 | 2.8923 | 3.0238 |
| `scheduler_uncontended` | 100 | 0.0007 | 0.0044 |
| `read_tool_dispatch` | 30 | 0.7985 | 1.2912 |
| `context_compile_201_messages_2550000_text_chars` | 10 | 83.3646 | 91.5104 |
| `context_compile_already_compiled` | 30 | 0.1722 | 0.1872 |
| `event_stream_10000_events` | 10 | 1.7310 | 4.6694 |

Compiler workload: 201 synthetic messages with 2,550,000 tool-result text characters, contextWindow 32,768 and fixedTokens 1,200. The measured first-pass path includes cloning the input and artifact publication/indexing. Example reduction: 852,298 estimated tokens to 26,868, with 97 externalized results and 97 pruned catalog rounds. Target 20,316, reported over-target 6,552. Protected recent evidence prevents meeting the soft target; this is **not** a successful hard-budget admission demonstration. Warm already-compiled work is much cheaper. Use this distinction when prioritizing optimization.

The aggregate tool-cap, corrupt memory and source text/revision race probes reproduced B01–B03; they are correctness evidence, not performance measurements.

Ten fresh Node CLI `--version` processes completed successfully. Median wall time approximately 767.1 ms, range 694.4–781.0 ms; maximum child RSS reported across the runs 219,576 KiB (214.4 MiB). This measures imported CLI startup and version argument handling, including a startup banner. It excludes model discovery/load, session initialization, renderer/TUI readiness and true cold disk cache. Raw wall/user/system samples are in `evidence/ag1-cli-startup.json`. GNU `/usr/bin/time` was absent; Python subprocess/resource measured wall/CPU and aggregate maximum child RSS instead.

The focused coding-agent test run passed 465 tests across 43 files (14.31s between earliest test-file start and latest completion); the separate Godot suite passed 26 more. Suite names and test timings are in `evidence/ag1-tests.json` and `ag1-godot-tests.json`; `ag1-selected-tests.txt` makes the coding-agent selection explicit. Test execution times are not subsystem latency benchmarks. Build logs are retained. Do not compare TypeScript test-runner wall time to a Rust microbenchmark.

## Coverage, omissions and exact future experiments

| Required area | Current evidence | Future reproducible experiment / metric |
|---|---|---|
| Cold startup | Fresh-process CLI metadata only | 20 packaged launches per OS; measure exec→core-ready→first interactive frame, both warm cache and recorded cold-cache setup; include reference model-disabled and already-running-model cases |
| Idle RAM/CPU/process count | Post-work harness RSS/CPU only; not application idle | Open empty project, settle 60s, sample 5 min; sum PSS/private working sets across shell/core/worker/webviews, record CPU core-percent and process tree; model and project processes reported separately |
| Context compilation | First/steady synthetic compiler measured | Freeze real request fixtures at 32k/90k/large context and 2–3M externalized tokens; compare selection and first/steady p50/p95, allocated bytes and blocking time |
| Retrieval/freshness | Exact-symbol sourceContext, hash and warm validator measured | Exact-symbol, vague-follow-up, changed-file, forbidden/stale cases in small/10k/100k-file repos; record quality gates plus p95, bytes read and index cost |
| Artifact lookup/rehydration | get, 8KiB page, repeated ~965KiB put, lexical search measured | 1k/100k/1M records, UTF-8 pages, binary integrity and cold/warm index; latency/bytes/copied buffers, recovery time |
| Persistence/resume | async custom append + small journal reopen measured | 10k/100k-entry branches, competing writers/importer and fsync settings; durable-ack p95 and cold resume; fault matrix at every commit boundary |
| Git status/diff | Small clean fixture measured | Dirty/untracked/submodule/worktree/large repo; p95 and event-loop blocking; external concurrent changes |
| Index/search | 120-file uncached index measured | Controlled file counts/bytes/ignore rules; incremental edit/rename/delete; watcher overflow rebuild and idle wakeups |
| Scheduler | Uncontended permit measured; queue/cancel correctness tested | Multiple sessions/endpoint queues under fake streams; queue wait/fairness/cancel/reconcile costs and bounded queued bytes |
| Tool dispatch | Read tool production call measured | No-op tool runtime dispatch separately from disk I/O, parallel read and conflicting writes, policy/schema/approval latency |
| Model streaming | None live; synthetic EventStream throughput only | Deterministic fake SSE with text/reasoning/tool fragments at fixed rates + actual local/remote stream; TTFT, p95 display lag, bytes copied, slow-consumer queue depth |
| Long-session growth | Not a duration baseline | 8h and 24h synthetic/live repeated tasks with 2–3M externalized tokens, resume and file revisions; heap/PSS/disk slopes, evidence recall and success |
| Crash/recovery | Existing focused fault/retry tests; B02/B03 probes | Kill worker/host/model during admitted tool, write, blob publish, task update and import; recovery outcome/time and no duplicate external effects |
| Shutdown | Reference unit tests; no complete desktop process tree measured | Start model/dev-server/PTY/jobs, close window then explicit quit; time to settle/reap, owned child count, intentional adopted-service policy |
| Replay/evaluation | Deterministic replay tests passed | Same fixture/trial order across TS/Rust; report verifier task success, required evidence and runtime; separate harness overhead from model latency |

Live model benchmarks in `scripts/long-horizon-context-benchmark.ts`, `context-intelligence-benchmark.ts`, `repository-context-benchmark.ts`, `relayforge-long-horizon-benchmark.ts`, `swebench-organic-benchmark.ts` and `llamacpp-speed-probe.ts` are identified but not run. They can start/manage models or require configured endpoints/weights and long-lived external verifier environments. Run them later with explicit isolated configuration, fixed model/provider/build, fresh-prefix settings and preserved reports. Do not substitute a static retrieval score for live task success.

## Dyad baseline

Direct Dyad runtime metrics were not collected. The checkout has no installed Electron/native dependency build; its declared Node support is `>=24 <26` while the environment has Node 26.7.0, and Xvfb is unavailable. Installing/running a mismatched native stack would not produce a trustworthy packaged baseline. No absolute idle RAM, GPU, startup or installed-size number is inferred from Electron's reputation.

Use the pinned revision on supported Node 24/25 and a real desktop or correctly configured headless display. Build with lockfile and platform-native dependencies. Measure packaged app executable/install/archive sizes separately, not `du` of the source checkout. Scenarios: empty home; one project idle; streaming 1,000 messages; Monaco file/diff opened; preview running; settings/dialogs. Record process tree/PSS/private memory, CPU, startup milestones, frame latency/long tasks, GPU process/VRAM where attributable, background requests and open handles. Linux GPU metrics may not attribute shared webview memory cleanly; report that limitation. Repeat identical scenarios for Tauri, with identical project/model services or those services excluded and separately itemized.

## Proposed performance gates (not achievements)

The planner should adopt or refine these provisional budgets after complete app baselines exist:

- Shell idle memory at least 30% below equivalent packaged Dyad on each supported OS, measured across all shell/core/bridge/webview processes; aim for 50%. Report both temporary TS bridge and final Rust product.
- Idle CPU below 0.5% of one core over five minutes without project/model work; no continuous polling merely to render a status panel.
- Packaged startup p95 below 1.5s on the reference desktop with no model launch; UI responsive while models load separately. Model-ready latency has its own budget.
- Final shell install footprint at least 50% below matched Dyad, separating OS webview prerequisites, optional browsers, model weights and compilers. Do not claim fixed process count across platforms.
- Context/retrieval/persistence p95 must not regress more than 10% versus equivalent TS fixtures after noise characterization, while all correctness gates pass; target warm compiler <2ms and small-fixture source retrieval <25ms as initial alarms, not universal SLOs.
- Streaming display latency p95 below 50ms under controlled fake-stream load, bounded queue memory during a 30s paused renderer and successful resnapshot afterward.
- Zero leaked owned child processes after explicit stop/quit; cancellation settles within a documented per-resource deadline. Model-load cancellation may have a separate bound.
- No monotonic unbounded heap growth across repeated steady-state tasks; fit memory slope after warmup and specify allowed persistent artifact growth separately. Run 8h/24h gates before retiring the reference for long-horizon use.

Likely optimization candidates supported by source and measurements: heavy eager CLI imports, synchronous first-pass artifact/index work, repeated lexical/index construction, full-log tail reads, full partial-message clones and unbounded transport queues. Measure allocations/CPU profiles before arenas, custom allocators, lock-free designs or zero-copy claims. Correctness, deterministic retrieval and recoverable evidence outrank attractive benchmark numbers.
