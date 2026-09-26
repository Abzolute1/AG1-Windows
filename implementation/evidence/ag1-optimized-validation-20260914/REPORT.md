# Optimized Qwen3.6 backend through the full AG1 desktop

Completed 14 September 2026. The optimized backend successfully ran inside the actual AG1 desktop, including manual loading, chat, new-chat model reuse, approved file editing, command execution, web search/fetch, cancellation and recovery. It was faster in each observed task. This is a bounded integration comparison, not a statistical speed or quality guarantee.

The production AG1 runtime remains the original. No systemwide installation or configuration cutover was performed.

## Actual application and isolation

Both arms used the same current-source AG1 Tauri/WebKit desktop, built in release mode with `visual-test` to expose its private control broker. The renderer, native Host, runtime supervisor, inference provider and tools were real; `fixture_provider` was false. This was not the untouched installed desktop executable, a browser-only renderer, or a direct HTTP benchmark. [Build command, executable hash and renderer inputs](desktop-build.json), [build log](desktop-build.log).

The app opened an actual workspace through its folder dialog. Composer submissions and Load, New chat and Stop controls went through the real interface. Native approval bindings were accepted for the requested fixture edit and checker command. A private authenticated Xvfb display, separate XDG/app state and a bubblewrap-isolated authority registry kept the test separate from the user's AG1 session. No screenshots or user-screen captures were taken. DOM/native-state checks do not establish pixel-level visual correctness.

Only one model ran at a time on the RTX 4080. The scored optimized sequence ran before the original sequence; a separate optimized lifecycle run followed. Compilation and preservation hashing happened outside inference. The desktop retained ordinary thinking and sampling behavior; answers and tool trajectories were not forced to be identical. Network work and approximately one-second completion polling are included in task times.

## Observed desktop timings

One scored sequence per arm; these are measured elapsed times, not medians or token throughput.

| Action | Original | Optimized |
|---|---:|---:|
| Manual Load, including AG1 file verification | 19.072 s | 14.559 s |
| Chat: compute 17 × 19 and return JSON | 30.091 s | 8.040 s |
| Inspect, edit and verify a 120-record JSON file | 55.444 s | 21.267 s |
| Search, fetch official documentation and answer | 57.350 s | 17.116 s |

[Full scores and answers](scored-results.json), [original run](original/result.json), [optimized run](candidate-verified/result.json).

Both arms returned the correct chat JSON. Both changed only `svc-047.timeout_ms` from 450 to 875 in the 602-line fixture, preserving all other bytes and the checker. Both ran `python verify.py` successfully. Independent host-side exact-byte checks verified the result. The optimized model initially attempted an ambiguous edit matching eleven locations; AG1 refused that edit without writing, and the model retried successfully with more context. The retry is included in elapsed time.

Both arms performed one successful native `webSearch` and one successful native `webFetch`, cited the official Python page, and reached a completed native operation without pending approvals. The original also called `work`. Generated thinking, output lengths and tool paths differed, so the wall-time ratios cannot all be attributed to backend speed. For repeated, fixed-input prefill/decode/TTFT measurements, use the [earlier performance investigation](../llama-performance-20260914/REPORT.md).

Manual loading preserved the unsent draft and submitted no message. Opening a new chat retained the same app-owned backend PID. That PID stayed alive across the three scored tasks in each arm. Actual executable arguments and process memory mappings were captured: [original](original/actual-backend.json), [optimized](candidate-verified/actual-backend.json), [optimized library mappings](candidate-verified/backend-maps.txt). The optimized process mapped its standalone bundle libraries, with no original build-directory library dependency.

## Stop and recovery

In the separate optimized lifecycle test, Stop reached native idle in **0.508 seconds**, without an error or pending approval. AG1 terminated the owned model process for cancellation. The next request reloaded it and returned exactly `{"recovered":true,"answer":323}` in **10.246 seconds**, ending in a completed operation. The app's native `quit` command then exited normally, leaving no owned model process. [Lifecycle result](candidate-recovery/result.json), [independent response score](candidate-recovery/score.json).

The final DOM had no horizontal overflow at 1440 × 900 and showed Loaded. A visible “Response stopped / Cancelled” notice remained after recovery; this was recorded rather than interpreted as a backend error. The lifecycle result's `backend_survived_all_tasks: false` is expected because Stop terminated that process.

## Answer-quality limits

Web execution passed, but neither answer fully satisfied the requested answer quality. The original answer had 102 whitespace-delimited words and the optimized answer 108, exceeding the under-100-word instruction.

The original incorrectly described monotonic time as including sleep/hibernation on this Linux machine. The optimized answer incorrectly included daylight-saving changes among causes of `time.time()` jumping, and overstated the guarantee of a “correct” elapsed duration. Python distinguishes a monotonic clock from a suspend-inclusive clock; daylight-saving conversion is separate from seconds since the epoch. [Official Python time documentation](https://docs.python.org/3/library/time.html).

These are factual/instruction-following failures in generated answers despite successful tools. One answer per arm cannot establish a quality regression caused by the optimized kernels or speculation. This test does not certify hallucination-free research, long-context saturation, image inputs, multi-model switching, endurance, or all AG1 workflows.

## Exact runtime identity

The read-only model was **Qwen3.6-35B-A3B UD-Q4_K_XL**, using the existing BF16 projector. Both profiles retained context 90,000 (server allocation rounds to 90,112), q8_0 K/V cache, Flash Attention, one slot, 99 requested GPU layers, unified KV, Jinja and preserved reasoning.

| Setting | Original | Optimized |
|---|---|---|
| Server | Original `llama-cpp-turboquant/build/bin/llama-server` | Separate `selected-rows32-bundle/bundle-v1/bin/llama-server` |
| Source base | `bd1bf025fc55ffa1fcb2ba6d8bb8805f35671d1f` | Same base with isolated performance patches from prior investigation |
| Batch / microbatch | 256 / 128 | 4096 / 4096 |
| Decode / batch threads | 12 / 12 | 6 / 12 |
| CPU MoE layers | 20 | 32 |
| Load mode | none | mmap |
| Expert cache | Original behavior | 4480 MiB budget, selected staging/rows32 environment |
| Speculation | None | ngram-mod, minimum 2, maximum 16 |

Every argument and selected environment variable is in [original-launch.json](original-launch.json) and [candidate-launch.json](candidate-launch.json), including capture hashes. The private loopback port was 18436. AG1 derived candidate decode CPU range `0-5` from its six threads; the standalone benchmark used `0-11`. Prior source/runtime investigation found this range flag ineffective in the tested OpenMP server path, but the difference is explicitly retained. Raw inherited environment/configuration captures remain private because they can contain unrelated credentials.

## Reproduction and evidence

The [archived scripts](reproduction/) preserve configuration preparation, private desktop admission, actual UI driving, scoring and preservation audit. They contain machine-specific paths and are historical commands, not an idempotent rerun command. Select fresh scratch/evidence/workspace roots before rerunning; do not overwrite these runs. Reconcile installed helpers and capture hashes with the current machine first.

The sequence is: prepare two isolated sealed profiles using `prepare.py`; build/copy the control-enabled desktop and helper; admit isolated state with `prepare-desktop.py`; prepare the candidate label with `prepare-repeat.py`; run `desktop-run.py candidate --label candidate-verified` and `desktop-run.py original` sequentially; prepare/run the recovery label with `prepare-recovery.py` and `desktop-run.py candidate --label candidate-recovery --lifecycle`; score full persisted journals with `score.py`; audit protected material with `audit.py`. The scripts record the exact prompts and fixture. The original scratch root is `/home/alex/Projects/AG1/.local/ag1-optimized-validation-20260914`; private desktop workspaces are under `.local/p14-long-horizon/ag1perf-20260914`.

Per-run directories contain native views, visible DOM text, complete task journal messages, approvals, fixture bytes and runtime logs. Runtime timing lines are retained as diagnostic evidence, but unmatched intermediate generations are not presented as paired throughput results. This desktop run did not independently sample CPU/GPU utilization or peak RAM/VRAM; those measurements belong to the earlier controlled investigation.

Excluded automation attempts remain intact:

- `candidate`: the first Load click occurred before the control became enabled; a later correction loaded successfully. The edit then paused at a genuine approval that the initial driver did not handle. Its apparent load time and incomplete tool sequence are excluded.
- `candidate-lifecycle`: Stop worked, but automation attempted to submit before React restored the Send control. Waiting for an enabled Send control fixed the driver; `candidate-recovery` is the completed repeat.
- The initial driver used a nonexistent quit-command name for the scored sequences, then terminated its owned namespace. Those runs do not prove graceful application shutdown; the completed recovery run does.
- Native views retain a bounded message window. Initial message-count deltas and first-assistant-block timings are not reliable token/TTFT metrics. Final scoring uses the complete persisted session journal; raw observations were preserved.

## Preservation

The final SHA-256 audit found **all 114 protected runtime/configuration files unchanged**, the **model and projector unchanged**, and the **original source commit, worktree status and remotes unchanged**. Installed `/opt/ag1/current` still resolves to `/opt/ag1/releases/ee7da595f62146489bc06da06be93e10`.

[Runtime/configuration audit](protected-after.json), [model/projector audit](model-materials-after.json), [original source audit](original-git-after.json). The optimized build is validated for these real AG1 workflows and remains separate; the installed production profile still selects the original backend.
