# P01 — Reference, runtime lock and executable evidence

## Objective and reading

Make future parity claims reproducible before new runtime work. Read the common contract in `phases/README.md`, all research documents, `planning/LLAMA_CPP_PRESERVATION.md`, and `planning/COVERAGE.md`. Primary scope: A32,A76,A77,A81 and the fixture inventory for all A01–A85; I28–I30; L01–L03,L07,L08. Inspect AG1 `llamacpp-runtime.ts`, `inference-scheduler.ts`, provider registration, package exports and the existing defect harnesses.

## Entry and allowed work

Requires the completed research files and pinned source checkouts, not a pre-existing product. Verify their HEADs, original source hashes/status, all three runtime paths and the profile hash. Preserve pre-existing modifications. Allowed outputs: `reference/`, `fixtures/`, `tools/`, `implementation/evidence/P01/`, license manifests and local ignored runtime manifests. No production application scaffold yet. No changes to originals, user profiles, model processes or backend builds.

## Work and authority

1. Create an immutable pinned source snapshot and a separately rebuildable executable reference. Copy dependencies/install into the copy only; keep MIT notices. Record exact toolchain/lockfiles. Run the 465 selected coding-agent and 26 Godot tests from research in this isolated tree. Preserve baseline failures honestly if the environment changes.
2. Create fixture schemas for normalized messages/events, clocks/IDs, source bytes, Git state, artifacts, task/episode/memory and provider streams. Capture one passing trace plus a failure case for every high-risk row. Refine broad rows into named child cases: all registered provider families/auth paths, each selected mutation tool, every persisted format version, mode actions and exported extension interfaces. Fixtures with secrets use fake values and privately held origin references.
3. Freeze corrected-behavior assertions for B01–B04. Run the original probes to show the reference violates them. Mark those outcomes as expected known defects; do not make the future Rust assertion expect the defect.
4. Complete the runtime manifest from the observed lock: executable and actual loader dependency identities, GGUF split set/projector identity, alias, explicit model selection, source/build metadata when available, context tier, effective argv/environment and request sampling/template/stop/cache settings. Use read-only inspection and a captured/fake spawn to resolve launch plans. Model bytes may be large: hash once and cache by validated identity; no weight copies needed. Keep secrets outside reports. Unknown build origins remain unknown. Do not run `ensureLlamaCppServer` with real process dependencies.
5. Create a Dyad candidate extraction ledger with pinned file hash, license/notice, intended destination and transitive imports/assets. Resolve fonts/icons from exact packages before approving copy. Record required visual scenarios and compatible Dyad runtime build instructions. No Pro/type/asset inclusion.
6. Create `tools/phase_gate.py` and `tools/reference_oracles.py`. These are test orchestration tools, not the new agent. The phase gate executes registered checks, checks input hashes/test counts/evidence and fails on missing dependencies or empty/skipped required selections. Future phases extend its registry. A `--check-plan` mode only validates documents and must never report implementation success.

## Interfaces and tests

Deliver `fixtures/manifest.json` mapping each parent/child ID to input files, source owner, expected observation and normalizations. Deliver `fixtures/llama/launch-plans.json` with all three configured models and boundary/absent-field cases; effective environment secrets stay private. Immutable reference remains sole behavior oracle; nothing takes ownership of real user state.

Commands this phase must make runnable from AG1 root:

```sh
python3 tools/reference_oracles.py --suite selected --offline
python3 tools/reference_oracles.py --suite known-defects --offline
python3 tools/phase_gate.py P01
```

Test the gate with an absent fixture, stale hash, zero selected cases and failed subprocess: all must fail. Confirm runtime-plan capture performs zero real spawn/kill/network requests. Verify model-specific binary precedence, Flash 90k ubatch tier, no priority when false and preserved ordered extraArgs. Capture argv/raw-byte identity, not merely profile JSON equality.

## Measurement and completion

Record baseline fixture latencies with hardware/cache/load scope and preserve raw samples. P04 owns actual packaged visual/idle baseline; P14 owns duration/live closure. Exit requires executable reference reports, defect probes, child coverage inventory, private/public runtime manifests, provenance ledger and a working gate. Missing model/loader facts must block later live-runtime equivalence, not offline fixture development; list the exact unresolved fields.

## Rollback and handoff

Delete only this phase's disposable copies if rejected; originals remain untouched. Deliver completion report and hash manifests. P02 can rely on actual fixtures and gate runner, P04 on reviewed copy candidates, P06 on measured/captured model-specific resolution. No claim of live tuned-runtime equivalence yet.
