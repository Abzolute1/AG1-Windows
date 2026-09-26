# P11 — Web, native, Tauri and Godot project targets

## Objective and reading

Deliver project create/load/files/install/build/run/preview/log/stop/history workflows across real target kinds. Read common instructions, TARGET_ARCHITECTURE project model, Dyad runtime/preview traces, AG1 check/visual tools and Godot protocol/host/staging tests. Scope A27,A45,A67,A69,A78; I07,I21,I25–I27; L03,L06,L07.

## Entry and allowed work

Require P10 runtime and P04 early preview/platform evidence. P03 process/PTY and P09 policy remain the only OS/tool ownership primitives. Implement platform target adapters, core target/diagnostic DTOs and desktop preview boundary in fixture projects. No Pro implementation, user repo rollback, model backend changes or embedded Chromium requirement.

For a new thread, first read the [P11 handoff](../implementation/evidence/implementation-records/phases/P11-HANDOFF.md): it records the verified P10 checkpoint, dependency check, concrete integration APIs, unfinished target/gate work and L01–L08 preservation requirements. Reverify its observations against the current checkout.

## Work and authority

Implement stable project/target IDs with kind, workspace, toolchain, capabilities and lifecycle. Provide web dev-server, generic native command, Rust/Tauri and existing Godot host adapters. Creation/import initializes only the requested new destination and cleans up failed creation; loading preserves project/session identity. Dependency install/build/run are auditable operations with stdout/stderr, readiness, cancellation and resource-group shutdown.

For Tauri discover frontend/package configuration plus src-tauri Cargo/config/capabilities/plugins; supervise frontend and Cargo dev processes as a group. Parse Cargo JSON and frontend/native crash diagnostics; expose missing toolchains and packaging target requirements. Compiler success is not runtime/visual proof. Generated applications never inherit AG1 secrets or commands.

Web preview uses isolated origin/session and constrained navigation from P04's proven path. Optional automation browser runs separately with bounded artifacts, URL policy and explicit capability status. Do not translate Electron WebContentsView/CDP APIs mechanically. Native preview means launch/focus/observe/stop plus diagnostic/artifact presentation unless actual embedding is proved on the platform. Unsupported visual actions are unavailable, not fake successes.

Preserve Godot version/hash/nonce handshake, host tool allowlist, scoped transactions, staging, parent watchdog and text/binary/runtime evidence. Adapt to Rust core without discarding existing protocol consumers. Vision attachment remains model/projector-capability gated and respects B04 artifact access/retention.

Implement Git/version preview with expected HEAD/dirty/conflict guards and recovery reporting. Keep Git commits separate from session branch rollback and native/database state. No automatic destructive restore merely because chat selected an old message. Register check/verify/visual tools against actual target capabilities through P09 policy.

## Interfaces, tests and commands

Deliver target discovery, install/build/run/stop/log/diagnostic/verification ports and protocol capability reporting. Reuse audited preview toolbar/console/problems/loading/error presentation candidates only; no Pro Annotator, billing/backend IPC or native overlay source copied blindly. Log all copied paths in the provenance ledger.

Create `project_runtime_contract` under ag1-platform plus Godot protocol adapter cases. Fixtures: tiny Vite app with deliberate compile/runtime failures; Rust/Tauri app with frontend error, Rust error and native crash; controlled generic native process; Godot fake host with handshake/transaction/watchdog errors and visual evidence. Test repeated start/stop, rename/delete while running, stale readiness, closed preview, child leak, foreign port, unavailable browser, dirty Git restore and no privilege escalation from generated app.

```sh
cargo test --manifest-path product/Cargo.toml -p ag1-platform --test project_runtime_contract
python3 tools/phase_gate.py P11
```

## Measurement and completion

Measure run-ready/log-display/stop timings separately from dependency download and compiler work. Report memory of optional browsers/targets separately and in total. Exit requires local workflows on development OS, Godot contract parity, actionable unsupported-platform results, no leaked owned child and native proof distinct from compile success. Platform capability gaps remain P14 gates.

## Rollback and handoff

Stop only owned fixture groups, retain user-generated state and record failed teardown. TS desktop stays available; no live authority transfer. P12 receives real runtime targets/capabilities to render, including truthful native preview limitations.

Write `implementation/evidence/implementation-records/phases/P12-HANDOFF.md` before finishing so the next thread has entry checks, completion evidence, delivered APIs, residual gaps and all L01–L08 constraints without relying on this conversation.
