# P08 — Retrieval, episodes and context compilation

## Objective and reading

Port AG1's context behavior on immutable captured inputs. Read common instructions, context compiler/feature switches/capsules/retrieval signals, episode modules, compaction family, prompt/addenda/prefix code and replay fixtures. Scope A15,A18–A21,A28–A35,A39,A76; I02,I05,I09,I12,I28; L03,L05,L08 constrain model-assisted operations.

## Entry and allowed work

Require P07 source/task/memory/evidence APIs, P06 isolated model call port, P05 persistence and P01 replay oracles. Modify core context/retrieval/episode/prompt modules and isolated fixtures only. Do not replace the executing TS compiler or pipe per-file compiler requests across the worker boundary.

## Work and authority

Implement explicit path selection, rg lexical/regex-symbol retrieval, bounded relationship assembly, recent-access and changed-file fallbacks with deterministic tie ordering. Preserve clipped markers and capsule provenance; Git HEAD is not source freshness. Cover exact-symbol, vague follow-up and changed-file recovery independently. No embeddings, learned reranker, general GC or new delta wire algorithm.

Port task/memory/runtime/inspection/episode request projections and stable system prefix. Preserve exact resource/addendum behavior and mode inputs through P10. Implement explicit episode begin/close with original trajectory, optional isolated same-model draft as non-authoritative, and automatic bounded pointers (max3/2400chars) with source/artifact/conflict checks. Model drafting uses the one scheduler, never a second ad hoc client.

Port deterministic compiler estimates, default ratio0.62, caps48k/160k, low water0.78, protected recent3 turns, evidence lease budget, reduction ordering and complete tool-call/result pairs. Externalize durable original data before shrinking projections. Preserve typed cursors/leases/segmented catalog defaults on and explicit feature overrides. Report protected over-target work rather than deleting decisive evidence to pass a budget test.

Implement manual/automatic summary preparation and emergency hygiene/fallback pure transformations. Their epoch commit/retry integration waits for P10; this phase cannot claim live admission correctness by testing only a static function. Preserve original trajectory/summary provenance and pending suffix transformation contracts.

## Interfaces and tests

Deliver `compileRequest(snapshot, budget, modelCapabilities)` returning projection, evidence/lease decisions, estimates and over-target diagnostics; retrieval/query and episode services consume explicit state, clock and provider ports. Compiler commits necessary artifacts through a controlled shadow transaction; input history remains recoverable. No renderer state or process cwd dependency.

Create `context_contract` and a differential replay runner. Use 32k/90k/large-window histories including the measured 201-message protected-over-target fixture and 2–3M externalized-token histories. Test required/forbidden retrieval, stale-authoritative claims, branch/worktree/source edits, capsule reconstruction, forged/expired cursors, memory caps, episode contradictions, exact trajectory rehydration and prefix stability. Normalize clocks/random IDs only, not selected evidence/order/claim kind. B03 correction may deliberately differ from reference; document that diff.

```sh
cargo test --manifest-path product/Cargo.toml -p ag1-core --test context_contract
python3 tools/reference_oracles.py --suite context-differential --offline
python3 tools/phase_gate.py P08
```

No Dyad copy. Provider/model calls use fixtures first; any live draft test follows L01–L08 and exclusive endpoint rules.

## Measurement and completion

Measure cold first compilation separately from already-compiled warm pass, allocations, I/O, retrieval quality, rehydration and retained tokens. Apply no >10% p95 regression only to equivalent correctness/durability workloads after noise characterization. Quality gates require zero forbidden/stale-authoritative regression regardless of token savings. Exit needs replay evidence for every listed capability, full selected evidence recoverable and explicit remaining live/epoch integration gates.

## Rollback and handoff

TS stays authoritative. Discard shadow outputs without changing original evidence. P10 receives a complete context pipeline with explicit commit/admission hooks and model-assisted call semantics; it must integrate persistence/epoch barriers before cutover.
