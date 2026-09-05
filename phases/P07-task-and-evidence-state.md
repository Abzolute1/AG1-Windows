# P07 — Task, memory and byte-consistent evidence

## Objective and reading

Deliver structured state and trustworthy source/artifact APIs before retrieval policy. Read common instructions, AG1 task-state, todos, memory-context, source-revision/sourceContext, runtime/inspection/delegation evidence, artifact tools, B03 and INVARIANTS. Scope A22–A27,A36 and state portions A38–A45; I06–I08,I10–I14; L07 protects imported runtime material.

## Entry and allowed work

Require P03 validated filesystem snapshots and P05 journal/artifact/memory storage. Use `product/crates/ag1-core` state/evidence modules, adapter extensions, `fixtures/evidence` and private test databases. No active TS state writes, live model calls or UI authority changes.

## Work and authority

Implement task v1 reducers preserving objective separately from latest instruction, phase/status/constraints/decisions and bounded epistemic entries. Retain FACT/OBSERVATION/HYPOTHESIS/DECISION/CONSTRAINT/ASSUMPTION/RESULT/FAILURE, provenance/confidence and explicit mutation validation. Preserve list/text caps and migration interpretation. Todo/plan/inspection/runtime/checkpoint records remain distinct. Checkpoint passing-check is narrow compilation evidence; later relevant source mutation invalidates it. Do not promote job exit/LLM claims into requested runtime/visual proof.

Implement project memory persistence commands and deterministic selection with core/normal, kinds, source metadata, hot cap40, archive and bounded injection16/~1600chars. Core priority does not erase caps; stale labels and recent filler exclusions remain. Full prompt insertion comes P08. User edits go through transactions; no parallel UI memory store.

Correct B03 by deriving raw bytes and digest from one validated source snapshot. Preserve realpath containment, regular-file checks, canonical `file:path@sha256` identity, same-size/rename/symlink invalidation and current/stale/missing/invalid/unverified distinctions. Hash raw full bytes before normalized display clipping; explain truncation separately. Tool read and hashing must not be two independently timed reads.

Build evidence read/search/page/lease operations over P05 storage with explicit requester authorization, artifact provenance and binary capability metadata. Validate historical capsules/claims against current source without rewriting historical evidence. Introduce source-access and runtime-observation reducers consumed later by retrieval/runtime. Retain separate source revision and native target revision/proof.

## Interfaces and tests

Deliver `TaskStateService`, memory command/selection API, `SourceSnapshot`/freshness validator, observation and evidence reducers or equivalents. Storage owns commit; core owns validation/selection. These shadow services run only on captured inputs. No frontend reuse.

Create `evidence_contract` and source snapshot race integration cases. Test objective continuity through steering/resume, invalid claim promotion, passing compile followed by edit/runtime failure, stale memories, archive migration, evidence paging/cross-session errors and all B03 variants: edit after read, replacement/rename, symlink swap, same-size changes, CRLF/BOM/Unicode. A stale source may remain historical evidence but never gets a current label accidentally.

```sh
cargo test --manifest-path product/Cargo.toml -p ag1-core --test evidence_contract
python3 tools/phase_gate.py P07
```

Use an injected mutation barrier to force the race deterministically; a timing-based stress test alone is not the regression oracle. P05 B02/B04 tests must still pass if state/storage interfaces change.

## Measurement and completion

Measure snapshot/hash/cached validation and bounded state serialization on small/large inputs. Compare identical filesystem/cache conditions; no metadata-only shortcut may weaken freshness. Exit requires task/claim/memory parity, fixed text/hash consistency, typed evidence API and restart tests with artifacts/cursors intact.

## Rollback and handoff

Discard shadow databases/caches if rejected; existing desktop remains TS-owned. P08 can build deterministic retrieval and compiler against verified evidence. P09/P10 can consume transactionally valid state without creating alternate reducers.
