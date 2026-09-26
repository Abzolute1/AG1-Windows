# Final overnight fix follow-up — 2026-09-13

The user authorized continuing all remaining fixes, short GPU validation, closing
Discord, and systemwide installation. Firefox must remain open. Do not ask for
those permissions again. No eight-hour run is authorized. Use small checkpoint
checks and one broader final pass. Preserve the dirty workspace and older receipts.

Evidence: `implementation/evidence/overnight-final-followup-20260913/` (E).
Earlier delivered changes are recorded in `HANDOFF-OVERNIGHT-FIXES-20260913.md`,
`HANDOFF-OVERNIGHT-FIXRUN-20260913.md`, and
`HANDOFF-OVERNIGHT-REQUEST-CUSTODY-20260913.md`.

## Final implementation changes

Working-set extension formerly hashed each entire old prefix for validation and
then serialized it again while constructing the next cache. It now verifies the
complete old array digest and continues the same hash state with the suffix.
The prospective next cache carries sealed hashes and immutable borrows of the
validated inputs until publication. Owner/epoch, configuration, exact branch,
source freshness and final admission checks remain. No prefix bytes are skipped.
Compiled messages still have the copies needed by the current owned compiler API.

The retained-history probe uses the actual old/new implementations on eight
snapshots, three pairs each. Median extension plus next-cache construction fell
from 9.831 ms to 5.788 ms (~41%). Future reuse produces identical values across
all pairs. Both arms charge the same compiler-output copy; actual compilation is
excluded. Allocation counts/peak allocation are unchanged in this probe. This
is a CPU-stage result, not a whole-request or model latency claim. Two focused
hash/continuation regressions passed. The initial borrow-checker failure and its
correction are retained, along with exact source backups and patches.

The first short model exercise exposed an additional native lifecycle defect:
`TurnLease::drop` cancelled the parent command even after a successful turn.
Ordinary one-turn commands could settle, but print recovery re-entered with an
already-cancelled token. The baseline model repaired the fixture and its original
verifier passed, yet the command reported cancelled when completion recovery was
needed. A wire-level regression reproduced this failure without a model.

Successful turns now drain their transport/tool work without setting the shared
command cancellation flag. Error and abandoned-future paths retain cancellation;
existing cancellation flags are never cleared. The regression proves both a
successful two-turn continuation and all three completion recoveries ending in a
real failed (unverified) command rather than false cancellation. It failed before
and passed after. The full native session target includes cancellation, dropped
future, receipt, planning and recovery coverage.

The next live check confirmed recovery now runs, but exposed confusing verifier
feedback: the model repeatedly prepended `cd ... &&` to the trusted command. The
strict ledger did not recognize that shell composition and eventually failed the
bounded command. Its positive test output is retained separately from the failed
native receipt. The final change explains the exact mismatch in request-local
status and tells the model to submit the recorded command unchanged: Bash already
starts in the workspace. Verification recognition, mutation invalidation and
receipt authority are unchanged. A real native-shell regression proves a wrapped
passing command still leaves verification required, followed by an exact check
that passes and expires the mismatch explanation. The contract text remains in
the transient tail, preserving the stable system prefix.

The preferred-tool exercise then exposed an unclear action distinction: the model
sent `work action=update` with an objective but no entry ID twice. The final facade
description now distinguishes `set` for objective/phase/nextAction from `update`
for an existing claim entry. A missing-ID update receives a `work`-specific
correction before dispatch. No invalid request is silently converted or executed.
The full six-test native-surface target passed, including real task-state reduction
for the corrected set action and preservation of valid entry-update arguments.

## Verification

Checkpoint checks were limited to the two hashing regressions, the reproduced
wire lifecycle defect and the wrapped-verifier feedback case. After the final
feedback change, the final native pass succeeded:
268 session tests, nine working-set contract tests and three client-guidance tests
(**280 total**). Production packaging and the isolated checksum-verifying installer
passed. The earlier 278-test candidate and package are retained under
`E/before-lifecycle-fix/`; they are not the final build. The subsequent 279-test lifecycle candidate is retained under
`E/before-verifier-feedback/`. The 280-test candidate before the final work-facade
clarification is retained under `E/before-work-feedback/`. That last change passed
the complete six-test native-surface target; the unchanged native session targets
were not repeated. Final packaging uses the final facade sources. Renderer code was unchanged
in this follow-up, so the previous production/WebKit MCP upgrade checks were not
repeated. Source fingerprints remained unchanged during the final build.

LIVE_REVIEW_PENDING

INSTALLATION_PENDING

## Queue disposition and limits

The original diagnosed implementation queue is delivered across the linked
handoffs: stable ordinary-print verifier tail, native verification status,
versioned lossless custody compression, safe missing-Git-info handling, renderer
containment, correct tool-artifact timestamps, selected `work` guidance, preferred
archive evidence wording, borrowed request-custody serialization, and now redundant
working-set hash removal. The newly observed command-recovery cancellation is
also fixed. These are implementation/short-validation results, not a guarantee of
model truthfulness or indefinite memory/disk bounds.

Renderer containment retains loaded DOM/React state. Custody compression preserves
all logical bytes but does not stop durable history growing. Controlled policy
projection has its separate system text; the ordinary-print cache finding does not
claim all mode prefixes are stable. Cross-model quality and a duration acceptance
matrix are separate future validation, not an outstanding diagnosed code fix.

Run9 remains stopped at 2.2389903826 hours, with 58 verifier passes and 57/58 semantic
reviews passed. No prior failure was relabelled, no duration controller was started,
and no P14 or eight-hour acceptance is claimed. Preserve protected profiles,
runtime capture and external model supervisor. Use `.local/p14-target`, six jobs
and the retained Node 24 toolchain for subsequent builds. No commit, reset, stash,
external MCP connection, GitHub write or forced user-app restart was performed.
