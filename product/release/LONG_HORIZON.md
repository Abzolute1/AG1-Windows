# P14 long-horizon evaluation

The 8-hour and 24-hour gates are repeated-session endurance workloads. They must
produce externally checked repairs, retrieval and recovery evidence throughout
the measured interval. Repeated prompts, an idle process or a final successful
answer cannot substitute for those observations. Neither duration gate by itself
establishes autonomous completion of one complex project over that entire time.

Legacy AG1 is the regression comparator. Improvement beyond that baseline needs
separate evidence of task success, current-source reasoning, recall, avoidance of
obsolete evidence and recovery, particularly on the preserved Qwen3.6 35B-A3B
configuration. Shell memory or CPU improvements do not establish model coherence.

## Current preparation

`tools/release_long_horizon_prepare.mjs NEW_PRIVATE_DIRECTORY` extracts original
scenario functions, checkpoint verifiers and the checkpoint task expression from
the immutable reference benchmark. It records their hashes and executes positive
and negative verifier controls. Generated functions receive their session and
tokenization adapters explicitly. Extraction does not start inference.

`tools/release_long_horizon_rehearsal.mjs PREPARED_DIRECTORY SEED_REPORT`
executes all six original checkpoint evaluators through real TS sessions using
scripted provider responses. It checks artifact retrieval, source-edit scope,
unchanged verifier bytes, active-context limits and journal reopening. Its report
always identifies this as an offline rehearsal with zero live model actions.

The native preparation also exercises frozen history import and a real Host
session. A copied journal must explicitly name its disposable destination
workspace before native resume; preserve the source journal and record the
derived header mapping. Check the selected session and command terminal outcome,
then check the exact contract in the actual artifact tool result. Successful
dispatch or a successful scripted edit alone does not prove resume or retrieval.

See `implementation/evidence/P14/long-horizon-preparation-review.json` for the
retained observations and limitations. No duration acceptance follows from these
rehearsals. The live owner integration and full duration workload remain pending.

## Required live admission

Before starting the endurance interval:

- Acquire the shared native runtime lease and verify the unchanged captured
  materials, settings, source inputs and authenticated process supervisor.
- Calibrate and count the original externalized history using the admitted
  model's tokenizer. Require 2–3 million distinct externalized tokens; an
  estimated character conversion cannot establish this requirement.
- Run the original live repair checkpoints sequentially in disposable TS/Rust
  workspaces, retaining requests, tool results, usage, verifier outcomes, diffs
  and artifact identity checks. TS runtime preparation must delegate to the
  actual native owner; endpoint health or a copied marker grants no custody.
- Rehearse branch/fork/resume, actual renderer detach/reconnect, owned model/host
  crashes and corrupt-state recovery. Never inject faults into the migrated
  production state or use a subscriber-only test as renderer evidence.
- Freeze candidate/input hashes, memory sampling coverage and growth budgets,
  activity policy, recovery assertions and timing rules in the start receipt.

## Duration evidence

The validator requires a monotonic interval of at least 28,800 or 86,400 seconds,
real activity receipts covering that interval, all required fault scenarios and
minute-level resource measurements. It rejects duplicate activity IDs, overlaps,
excessive idle gaps, missing task work and an unbounded single activity used to
cover the entire run. Current policy allows at most 120 seconds idle between
activities, at most 1,800 seconds per activity and requires at least two successful
task activities per hour of requested duration.

The driver must record actual start/end times and verified outcomes; these limits
are acceptance constraints, not a scheduler that makes unsuccessful work pass.
A short, interrupted or failed run remains evidence of that outcome. Retain its
logs and start a separately identified attempt after any necessary repair.

Track process-tree resources and heap coverage explicitly, fit post-warmup memory
growth, and separate persistent journal/artifact growth from live allocations.
Preserve raw measurements and failed checkpoints. Report comparative task quality
separately from runtime overhead and shell performance.
