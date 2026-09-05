# P02 — Core boundary, protocol and fake host

## Objective and reading

Create the smallest testable Rust foundation and an explicit core/client contract. Read common phase instructions, DECISIONS D01–D03, TARGET_ARCHITECTURE protocol/state sections, SDK/RPC/Godot source seams and P01 fixture schemas. Scope: foundations for A02,A26,A41,A42,A59,A60,A73,A75,A78; I01,I11,I20,I22; L05–L07 constrain later adapters.

## Entry and allowed work

Require P01 completion, `fixtures/manifest.json`, runtime launch fixtures and working gate. Create `product/Cargo.toml`, core/protocol packages and empty composition package boundaries only as needed by real tests; no placeholders claiming storage or tools exist. Allowed: `product/crates/ag1-core`, `ag1-protocol`, protocol fixture harnesses, `tools/phase_gate.py` registry and P02 evidence. No real model, tool execution, persistence migration or copied UI.

## Work and authority

Define strong workspace/session/branch/operation/artifact IDs, state/error views and core port traits. Keep core independent from Tauri/storage/platform. Define versioned commands, results, snapshots, durable event envelopes, transient progress, subscriptions, cancellation, approvals and bounded artifact reads. Use decimal strings for sequences crossing JS; retain explicit optional versus null semantics and opaque custom payloads.

Select/pin a Rust DTO → JSON Schema/TypeScript generation path by actually compiling fixture unions/errors in Rust and TS. Store generated output and make drift a check failure. The protocol owns representation only. Build a fake host with in-memory fixture state, deterministic clock and controllable scheduler; label it fake. It must exercise the contract, not impersonate a model or real persistence.

Implement snapshot+subscription barrier, replay/gap/resync behavior, instance/epoch changes and idempotent accepted-command results. Implement the initial bounded admission policy in DECISIONS, including large-prompt upload/artifact references. No GUI consumer can force unbounded buffering; slow progress subscribers lose coalescible deltas with an explicit gap, while terminal state remains recoverable. Approval correlation validates argument/revision/epoch, even in fake mode.

## Interfaces and corrections

Deliver `product/crates/ag1-protocol/schema/`, generated `product/apps/desktop/src/generated/protocol.ts`, and `fixtures/protocol/`. Generated frontend file alone does not mean the desktop exists. Public command families cover project/session, prompt/queue/cancel, task/plan, files, artifacts/memory, model/runtime, diagnostics. Each unsupported fake operation returns a typed unavailable result, not success. P03/P04 implement selected families; later phases fill them with actual core ports.

## Tests and commands

Create integration test `protocol_contract` under ag1-protocol and a TS codec fixture checker runnable by the gate. Test >2^53 sequences, incompatible major/schema negotiation, out-of-order events, subscribe race, repeated commands, stale approval, restart, malformed/oversize JSON, queue saturation and 30-second paused subscriber. Fuzz envelope/cursor-like opaque input parsing with strict limits. Assert a late old-instance event cannot mutate a fresh snapshot and a repeated external-effect command returns prior admission rather than launches again.

```sh
cargo test --manifest-path product/Cargo.toml -p ag1-protocol --test protocol_contract
python3 tools/phase_gate.py P02
```

No frontend reuse is needed; any schema tool dependency needs its license recorded. Tests prove protocol semantics only, not filesystem durability or native IPC isolation.

## Measurement and completion

Record payload sizes, queue peak bytes and event dispatch at controlled rates. Require bounded RAM through paused subscribers and recovered terminal state; display latency awaits P04/P12. Exit requires compiling core/protocol, generated type round trips, deterministic fake host and passing race/limit tests, plus API docs usable by P03/P04. No actual data writer is introduced.

## Rollback and handoff

Only the new scaffold/generated files are affected. Revert the protocol version with its fixture/schema together if rejected. P03 receives concrete port traits; P04 receives a real fake host and transport contract, not assumptions about unbuilt services.
