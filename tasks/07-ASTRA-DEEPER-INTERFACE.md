# Astra interface — deeper fixes

Status: complete, installed systemwide and live verified on 2026-09-11. User explicitly requested continuing with the other fixes after the surgical pass. The [surgical pass](../implementation/evidence/implementation-records/desktop/ASTRA-SURGICAL-INTERFACE-20260911.md) is installed and verified with Luna and Qwen35B. MCP remains paused.

Implement incrementally, preserving historical journals, exact evidence, authority/approval boundaries and all tuned local model/CV settings:

1. Scope automatic evidence-lease release to an explicit verification obligation. Untagged/legacy leases must not be released by an unrelated passing check. Preserve atomic receipts, replay behavior and session isolation.
2. Expose a bounded model-accessible preview lifecycle through existing native Runtime ownership and grants; make eligible URLs discoverable without guessing or arbitrary browser access.
3. Simplify editing through a preferred facade with explicit preview/apply and one clear matching model, retaining old operation semantics for replay. Add revision-bound preview application only if needed to make the selected interface coherent.
4. Add convenient bounded evidence retrieval over existing storage: reference/query lookup, surrounding exact context, provenance and continuation under one budget.
5. Extend event/task-state projections to reduce duplicate records while preserving claims versus native verification and explicit task scope.
6. Carry exploration-board source/revision/age into retrieval and assess freshness before consequential reuse. Unify delegation presentation only where it simplifies the existing shared/isolated distinction; preserve child ownership and scheduler boundaries.

Todo remains the documented coarse sequential phase view; a new scheduler or status taxonomy is not needed merely to duplicate background jobs. Keep larger system rewrites out of scope when an additive interface suffices.

Relevant tests must cover unrelated verifier isolation, session/fork/import/replay, missing/ready previews and authority, stale edits, bounded retrieval/provenance, observation projections and stale board evidence. Build/install normal production checkpoints and retain reports. Use fresh Cargo profiles and preserved binaries; no subagents authorized. Screenshots, if needed, remain confined to authenticated private displays. Do not interrupt user-owned processes.

First design: an explicit named verification scope on lease/check, validated and captured by the native executor. Only a successful matching native check may release that scope's leases. It is a release obligation, not proof that the associated claim is true. Untagged checks/old leases remain conservative. Prefer storing the optional scope in existing lease metadata, avoiding schema migration if atomic isolation and import/fork preservation can be maintained.

Checkpoint: all six fixes are installed in `/opt/ag1/releases/fab3e5b44e114c51827453cdb3732951`. All 36 focused tests passed and all 1,142 installed payload files were verified. Luna completed the final installed-build workflow without tool errors; Qwen35B completed after correcting one missing lease reason. The original strict zero-error probe result is preserved alongside the recovery audit. See [implementation and evidence](../implementation/evidence/implementation-records/desktop/ASTRA-DEEPER-INTERFACE-20260911.md). Last weekly usage check: 7% remaining. No subagents or desktop capture; local/CV settings preserved. MCP remains paused.
