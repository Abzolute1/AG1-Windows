# P13 — Quiescent transfer to the Rust runtime

## Objective and reading

Move an explicitly selected workspace/session bundle from the TS bridge to Rust with one writer, preserved llama runtime and verified rollback. Read common instructions, DECISIONS D03/D04, LLAMA_CPP_PRESERVATION, storage/import/recovery risks and all previous completion evidence. Scope all active A01–A78 plus explicit A79–A85 dispositions; I01–I27; all L01–L08.

## Entry and allowed work

Require P12 and every transitive dependency passed, complete high-risk trace/failure coverage and B01–B04 corrections. First rehearse entirely on fixtures. Actual user-session migration must target a new destination and be within the explicitly requested implementation/migration scope; phase execution does not authorize changing Cruzible, backend builds or unknown user data. No pending mandatory offline gate may be waived by saying the UI works.

## Verified P12 starting state

Read `implementation/P13-HANDOFF.md`, the P12 report/completion, `product/clients/README.md`, `product/clients/COMPATIBILITY.md` and the SDK README. P12 delivered isolated native clients; ordinary desktop execution still belongs to the TS worker. Native desktop mode is selected once at startup by `AG1_NATIVE_CONFIG`, accepts only `.ag1-native-isolated` workspaces and uses separate private state. CLI state/configuration are explicit. The startup switch is not an authority-transfer protocol: P13 must deliver admission of migrated ordinary workspaces and connect desktop/CLI startup to the durable authority decision before changing defaults.

The native host currently accepts configured model catalogs and environment credential bindings. It has not imported the user's tuned profiles or credential vault, or transferred ownership of live llama processes. Map settings and credentials through the existing provider/runtime APIs with secret material kept private; preserve raw unset/false/zero values, per-model binary/environment precedence and ordered arguments/tiers. A host-local inference scheduler or a successful endpoint probe does not establish exclusive ownership across TS, Rust desktop and CLI processes.

Carry forward the explicit A01–A85 dispositions and P10/P11 capability boundaries. Migration does not make reference-only adjacent clients, dynamic extension provider rebind, native embedding or judge-only vision available. P12's fake-provider visual/renderer-freeze evidence does not satisfy P13 live per-model acceptance or P14 release/platform/long-duration gates.

## Transfer protocol

1. Mark the selected workspace draining. Reject new mutation/generation admission with retryable migration status; cancel or settle active operations under their policy. Persist accepted queues, approvals, journal/task/memory/episode/evidence and ambiguous external outcomes. Record the exact endpoint owner and runtime lock.
2. Verify no active tool/retry/compaction/subagent/job operation can mutate the transfer set. Long-lived owned jobs require explicit handoff with ownership token or settled stop; unverified external services stay external. Snapshot source files/Git identity separately from agent state; migration is not a worktree rollback.
3. Freeze a manifest of state, cursor keys, blobs, authorization roots, settings, pending decisions and schema versions. Copy/import through P05 into a new Rust destination and validate B04 closure, unknown records and source hashes. No live tail dual-write synchronization.
4. Start Rust in validation-only mode; compare imported snapshots/context/commands against reference, excluding documented corrected defects. Reconcile old process observations; never trust copied PIDs. Validate all three configured runtime plans against L01–L04 and the local manifest.
5. Revoke TS workspace execution and wait for its exit/drain before committing a single authority marker/lease to Rust. Atomically choose the new owner; recovery from a crash on either side must choose exactly one owner or remain safely unowned. Model scheduling switches with the runtime bundle, not before it.
6. Reconnect UI/CLI using new server instance/epoch and atomic snapshot watermark. Old messages/approvals cannot execute new work. Resume only clearly pending safe intents; externally ambiguous mutation remains interrupted/unknown for inspection.

Include P12 client-effect receipts, artifact custody and full journal detail behind bounded client pages in the transfer manifest and rollback closure. Preserve the distinction between `native-client-effect` and executable `native-session-command` intents. An uncertain file/export/upload/project/terminal effect must remain available for reconciliation and must never become a resumable session command. Drain owned project/terminal work and invalidate preview grants at the transfer boundary; a renderer disconnect alone does not stop the native host driver.

For a running tuned model: reuse only after full configuration identity and permitted control are established. A TS-owned process may need a controlled restart using the **same** pinned binary/libraries/argv/environment once exclusive ownership is transferred. A pre-existing user service remains externally owned and is not stopped by this phase. Port availability alone never proves identity or permission. No migration-triggered VRAM tuning or alternate backend fallback.

## Interfaces and rollback

Deliver a versioned migration manifest, durable authority marker, importer/exporter, start/drain/switch state machine and recovery UI/report. Rehearse rollback before accepting cutover. Freeze Rust, export compatible state to a **new** TS destination, preserve lossless extras/unknown records and cursor closure, restore exactly one owner and resnapshot clients. If a new feature cannot export safely, block downgrade and preserve both archives; do not reopen the original stale TS files and lose progress. Rollback does not undo worktree mutations or external effects.

## Tests and commands

Create desktop `cutover_contract`. Crash at every numbered boundary, repeat migration after restart, duplicate cutover command, race stale UI reply/tool timer, tamper manifest/profile/library, fork with old cursors, corrupt memory and reject unsupported export. Run a full prompt/tool/compaction/steering/resume workflow in the migrated Rust workspace. Assert zero concurrent writers/generations, no replayed external mutation, no lost accepted command and unchanged original/runtime hashes.

Exercise startup with both TS and native configurations, competing desktop/CLI opens, and recovery with a frozen or disconnected renderer. Verify that the authority decision fences every admission path, copied isolated-workspace markers cannot bypass transfer, ambiguous client receipts survive migration and rollback without replay, and artifact/detail pages remain complete after resnapshot. Assert expired previews cannot invoke AG1 commands or retain access to a transferred runtime.

```sh
cargo test --manifest-path product/Cargo.toml -p ag1-desktop --test cutover_contract
python3 tools/phase_gate.py P13
```

Per-model live canaries, when an endpoint window is available, compare locked TS/Rust configurations sequentially for exact-copy, reasoning, tool calls, context limits and cancellation. All three must pass before declaring the tuned-runtime cutover accepted. If unavailable, fixture cutover can be complete but real local-runtime acceptance stays pending and the previous usable owner remains available. Record this distinction in completion status.

## Measurement and completion

Record drain/import/resume and recovery time, active process ownership, launch manifest differences and context/quality parity. No numerical speed gain permits dropping data, features or L01–L08. Exit requires successful rehearsal/rollback, per-model acceptance for a real local cutover, one Rust authority and transparent compatibility status. TS bridge is retired from active execution, but reference copies/checkpoints remain until P14.

## Handoff

P14 receives the locally migrated product, exact runtime fingerprints, complete cutover/rollback reports and a tested recovery procedure. Packaging must retain the tuned runtime as an external dependency and must not reinstall it.
