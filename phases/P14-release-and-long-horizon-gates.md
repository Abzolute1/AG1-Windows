# P14 — Packaging, duration tests and acceptance

## Objective and reading

Produce a locally reviewable release candidate with measured parity, lightweight shell and long-horizon stability. Read common instructions, COVERAGE, PERFORMANCE_BASELINE, TEST_STRATEGY, LICENSE_AUDIT, LLAMA_CPP_PRESERVATION and P13 reports. Scope A76,A77 and final evidence for every A/I/B/L ID.

## Entry and allowed work

Requires accepted P13 cutover plus every prior implementation gate. Work on product packaging, deterministic/live evaluation drivers, CI/OS fixtures and reports. No public release/deployment is implied by building a candidate. Do not rebuild/upgrade/bundle-replace any existing tuned backend, copy private weights/credentials into artifacts or remove the reference to make install size look smaller.

## Work and authority

Package the Tauri/React/Rust product, optional extension/automation companions separately and only necessary runtime dependencies. Default package contains no Electron or complete TS AgentSession. Project Node/package managers and optional extension/browser runtimes are explicitly scoped external/optional dependencies. All state is Rust-owned; an extension host cannot retain a hidden parent loop/scheduler.

Installer/update logic references the user-installed per-model llama manifest; it must never write to the three original runtime directories or user profiles. Diagnose missing/incompatible tuned runtimes without stock fallback. Audit actual dependency/asset/binary closure with notices, SBOM, no Pro paths/types/implementations and no Dyad production telemetry/branding. Preserve applicable AG1/Dyad/third-party license texts. Treat bundled GPU/model payloads as separately reviewed material if later explicitly requested.

Run deterministic quality/retrieval/context/failure suites, then sequential live TS/Rust evaluation with the same locked model/backend/config/fixtures/cache conditions for every configured model. Use existing long-horizon, repository-context, context-intelligence and speed-probe scenarios; adapt their driver only, retaining verifier/evidence criteria. Record draft acceptance where exposed, not inferred. No simultaneous competing generation against the local endpoint.

Run both 8h and 24h repeated-session workloads with 2–3M externalized tokens, source edits, branch/fork/resume, window detach/reconnect, model/host crashes, corrupt-state recovery and preserved evidence. Fit post-warmup memory growth; distinguish expected persistent evidence growth from heap leak. Verify required recall, forbidden avoidance, contradiction handling, task success and zero stale-authoritative claims. Do not mark a shorter run as a duration pass.

Test packaged Windows/macOS/Linux shell/process/protocol/preview behavior using available supported automation plus recorded manual cases where needed. Tuned local model support initially applies to the inspected Linux builds; other OS model-runtime compatibility remains unavailable until separately authorized matching builds exist. Separate shell readiness from local-inference support, and explicitly retain pending OS runners instead of claiming cross-platform completion.

## Tests and commands

Extend the gate/eval drivers to perform actual packaging, artifact inspection and matched workloads:

```sh
cargo test --manifest-path product/Cargo.toml --workspace
npm --prefix product/apps/desktop run build
python3 tools/reference_oracles.py --suite release-differential --offline
python3 tools/phase_gate.py P14
```

The P14 gate must reference actual 8h/24h/live/platform report inputs with fingerprints and durations; it cannot synthesize success from their absence. Produce exact per-OS package/signing validation commands in the report; credentials/signing availability is a separate named gate. Test update interrupted mid-install, preserved external runtime paths, startup after crash, unknown pending operation and rollback archive readability.

## Performance and completion

Close matched packaged baselines: ≥30% lower shell idle memory versus Dyad, ≥50% smaller shell installation, idle CPU <0.5% of one core, startup p95 <1.5s excluding model load, display lag p95 <50ms, no >10% equivalent core p95 regression after noise analysis, bounded paused-subscriber queues and zero leaked owned children. Include all host/worker/webview processes; itemize optional models/browser/project runtimes and report combined totals. Backend tuning changes cannot count as a Rust optimization.

Exit requires every parent/child capability disposition and invariant backed by actual evidence, all four defect regressions, all eight llama requirements, clean provenance and accepted platform/long-duration metrics. Failed quality/performance or unavailable required live checks mean incomplete acceptance, not a smaller silent definition of done. List scoped platform availability honestly.

## Rollback and handoff

Retain P13 export/checkpoint and immutable reference reports. If release gates fail, keep the last proven owner or perform tested rollback; do not rewrite old data in place. Deliver packages, SBOM/notices, runtime manifest references, compatibility matrix, raw measurements and release-candidate report for review. Stop before publishing unless separately instructed.
