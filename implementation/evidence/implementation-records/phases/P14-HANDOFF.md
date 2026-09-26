# P14 handoff — preserve the accepted P13 authority and runtime

> Historical evidence, archived 2026-09-13. Status, commands, permissions and paths below describe their original checkpoint. For current work start with [implementation status](../../../README.md). Original bytes and old paths are preserved in the [relocation manifest](../manifest.json) and recovery archive.

> Historical P13-to-P14 entry handoff. P14 has since been implemented in part.
> **Current stop state and next-model instructions: [P14-RESUME-HANDOFF.md](P14-RESUME-HANDOFF.md).**
> The historical statements below that P14 has not started describe the original entry only.

This handoff does not start P14. Require `implementation/evidence/P13/completion.json` with status `passed`, no pending gates and a valid complete artifact manifest before beginning any dependent work. The P13 task stops after its completion commit. Read the final P13 REPORT, `product/clients/CUTOVER.md`, compatibility dispositions and all L01–L08 requirements first.

## Accepted entry and read-only verification

P13 completed in commit `fca1500`, with 46 cutover contracts, 767 native regressions, 36 live canaries and all 149 artifact references verified. Native authority was active at epoch 2 and quiescent. The updated [P14 plan](../../../../phases/P14-release-and-long-horizon-gates.md) carries forward the concrete build, recovery, performance and evidence boundaries. P14 implementation has not started; its gate and `release-differential` suite still need implementation.

Verify the accepted entry without rerunning phase gates that overwrite their reports:

```sh
git status --short
git log -3 --oneline
python3 - <<'PY'
import json
import sys
from pathlib import Path
sys.path.insert(0, 'tools')
from phase_gate import verify_dependencies
from cutover_gate import inputs
root = Path.cwd()
for phase in json.loads((root / 'planning/phase-graph.json').read_text()):
    verify_dependencies(phase)
p13 = json.loads((root / 'implementation/evidence/P13/completion.json').read_text())
assert p13['status'] == 'passed' and not p13['pending_gates']
assert p13['P14_started'] is False
assert inputs() == p13['input_fingerprints']['product'], 'Review changes since accepted P13 before proceeding'
print('P13 and prior artifact manifests verified; current inputs match accepted P13')
PY
sha256sum /home/alex/.ag1/agent/llamacpp-model-profiles.json product/target/release/ag1-process-host
product/target/release/ag1-cutover inspect /home/alex/Projects/Cruzible
```

The input equality check applies before P14 implementation changes. Preserve the accepted fingerprints and record subsequent regressions under P14; do not rewrite P13 to make new source appear historically tested. Inspect current process ownership and perform fresh runtime preservation verification before live admission. When the native owner is quiescent, `product/target/release/ag1-cutover audit-cutover /home/alex/Projects/Cruzible` performs the full read-only inspection; allow time for its material/archive hashing (the measured P13 audit took about nine minutes). Inspection does not grant execution control.

## Selected local state

The selected workspace is `/home/alex/Projects/Cruzible`, session `934360d3-bd97-4246-b48f-cee4addcf168`. Its durable per-user authority is under `/home/alex/.local/state/ag1/authority-v1`; native state is `/home/alex/Projects/AG1/.local/p13-production-20260907/native-state`. Ordinary desktop/CLI startup must obey that record. Do not create a second authority namespace or reopen the original TS state as a writer.

The source legacy state remains `/home/alex/.ag1/agent/sessions/--home-alex-Projects-Cruzible--`. Keep all original data and the entire private P13 production directory, including failed attempts and partial archives. `legacy-recovery-archive` retains the original agent state; `source-archive-3` separately retains the original worktree and Git identity. Source archives record literal links/overlay markers and never restore worktree files. The original Cruzible and Dyad pins remain `e30f0b4b238ee5618928bcd17b4571eb046e7038` and `fb17039bfd2adcac7d74b9c1178a3a3524df35b6`.

P13's full live matrix is `canary-cutover-7/canaries.json`: 36 sequential TS/Rust checks and 48 owned runtime events, all passed. Earlier failed matrices remain excluded from acceptance. `workflow-2` retains the successful real prompt/read/steering/compaction/reopen/stale-client/drain workflow. The final P13 certificate binds these observations, the actual renderer evidence and a fresh production authority audit. Private evidence is not a packaging input.

## Recovery boundary

`native-workflow-recovery` and `ts-workflow-recovery` retain a compatible native archive and newly prepared TS candidate. The real TS SDK resumed a disposable network-disabled copy and retained the native progress marker and unchanged selected journal. Production TS authority was not activated; the prepared rollback was cancelled to retain native ownership. Actual authority rollback and TS-worker execution were separately rehearsed on fixtures.

These are recovery checkpoints. Later native progress requires a fresh quiescent archive and new TS candidate through the documented freeze/prepare/commit protocol; do not activate an old checkpoint or reopen stale original journals. Unsupported native checkpoint/execution metadata must block downgrade while preserving both archives. Ambiguous client-effect receipts must never become resumable session commands. Archive restoration alone grants no execution authority.

## Runtime preservation

Keep `/home/alex/.ag1/agent/llamacpp-model-profiles.json` byte-for-byte at SHA-256 `aa02688c4d39ef73234f0ec381eed22dbe02463881626423049b75b5a12ad0d0`. Preserve all three existing tuned executables and their adjacent/loader libraries, weights, projectors, exact ordered arguments/environment and model settings:

- `/home/alex/Projects/llama-cpp-turboquant/build/bin/llama-server`
- `/home/alex/Projects/llama-cpp-qwen4exp-mtp-b10715/llama-server`
- `/home/alex/Projects/llama-cpp-qwen4exp/build/bin/llama-server`

The private capture is `.local/p13-runtime-capture-20260907/configuration.json`, SHA-256 `b6ea6f521e34c0278dd5d3c097ca2c83036eeebd52cb49305c4623734f3a9fba`. Its 111 material files total 124,130,526,787 bytes. Public plan fingerprints are in P13's `runtime-configuration-import.json`. No tuned runtime rebuild, upgrade, replacement, implicit tuning or profile write is authorized. Packaging must treat these runtimes as existing external dependencies and must not reinstall them.

The frozen native configuration also pins AG1's process supervisor at `product/target/release/ag1-process-host`, SHA-256 `e1c7ca1711b2b7834eea1c99a55fe958cb4cfb880ed3c6909b06232c36bc1da2`. Do not overwrite that bound executable with an incidental workspace build. Any future supervisor replacement needs an explicit quiescent configuration transition and verification; the current CLI/desktop use its unchanged protocol and authenticated ownership.

The old external model PID 1311504 and Discord were separately identified and gracefully stopped with explicit user authorization. Those receipts do not grant control over future external services. Runtime control remains limited to authenticated owned children; endpoint availability or a PID alone grants no stop authority.

## Remaining release scope

P13 is local migration acceptance, not packaged release acceptance. P14 retains its own packaging/provenance, platform, duration and performance/quality gates. Preserve P10/P11/P12 reference-only and unsupported capability boundaries, including native embedding, judge-only vision and dynamic extension-provider rebind. Keep reference copies and recovery checkpoints until the planned P14 retention boundary. Do not claim platform or long-duration results from the short P13 canaries or fixture renderer measurements.
