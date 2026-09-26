# P14 — Packaging, duration tests and acceptance

## Objective and reading

Produce a locally reviewable release candidate with measured parity, lightweight shell and long-horizon stability. Read common instructions, COVERAGE, PERFORMANCE_BASELINE, TEST_STRATEGY, LICENSE_AUDIT, LLAMA_CPP_PRESERVATION and P13 reports. Scope A76,A77 and final evidence for every A/I/B/L ID.

## Entry and allowed work

Requires accepted P13 cutover plus every prior implementation gate. Work on product packaging, deterministic/live evaluation drivers, CI/OS fixtures and reports. No public release/deployment is implied by building a candidate. Do not rebuild/upgrade/bundle-replace any existing tuned backend, copy private weights/credentials into artifacts or remove the reference to make install size look smaller.

## Accepted P13 starting point

Read [P14-HANDOFF](../implementation/evidence/implementation-records/phases/P14-HANDOFF.md), [P13 REPORT](../implementation/evidence/P13/REPORT.md), its `completion.json` and [CUTOVER](../product/clients/CUTOVER.md) before implementation. P13 was accepted in commit `fca1500`: no pending gates, 46 cutover contracts, 767 native regressions and 36 live TS/Rust canary checks across all three configured models under exclusive runtime ownership. The actual migrated workflow, compatible recovery inspection and 60-second production renderer freeze passed. These establish local cutover acceptance; they do not satisfy P14 duration, packaged performance, quality or platform gates.

The migrated workspace is `/home/alex/Projects/Cruzible`, session `934360d3-bd97-4246-b48f-cee4addcf168`. Native state is `/home/alex/Projects/AG1/.local/p13-production-20260907/native-state`; durable authority is under `/home/alex/.local/state/ag1/authority-v1`. At P13 completion the sole owner was native, active at epoch 2 and quiescent. Reinspect current authority before runtime admission; neither that historical epoch nor a copied marker grants a new lease. Do not repeat the original migration, reopen the original TS journals as a writer or create a second production authority namespace. P13 retirement applies to this selected workspace; other unselected legacy projects have not been migrated.

Verify every prior completion manifest read-only. Keep P01–P13 evidence immutable, including private artifacts referenced under `.local`; publish current regression results and new measurements under P14. Running the existing P13 gate in place rewrites its evidence, so it is not a read-only entry check. Use the handoff verification commands and retain accepted input fingerprints separately from subsequent P14 source changes.

P13 captured 111 runtime material files totaling 124,130,526,787 bytes. Preserve the tuned profile at SHA-256 `aa02688c4d39ef73234f0ec381eed22dbe02463881626423049b75b5a12ad0d0` and all exact per-model launch plans. The frozen native configuration also pins `product/target/release/ag1-process-host` at SHA-256 `e1c7ca1711b2b7834eea1c99a55fe958cb4cfb880ed3c6909b06232c36bc1da2`. Build release/package candidates in a separate target directory so a workspace build cannot overwrite this active dependency. A candidate supervisor needs its own tested configuration binding; adoption into the migrated state requires the documented quiescent transition. Keep original backend directories and private configuration outside package/update write targets.

## Work and authority

Package the Tauri/React/Rust product, optional extension/automation companions separately and only necessary runtime dependencies. Default package contains no Electron or complete TS AgentSession. Project Node/package managers and optional extension/browser runtimes are explicitly scoped external/optional dependencies. All state is Rust-owned; an extension host cannot retain a hidden parent loop/scheduler.

Installer/update logic references the user-installed per-model llama manifest; it must never write to the three original runtime directories or user profiles. Diagnose missing/incompatible tuned runtimes without stock fallback. Audit actual dependency/asset/binary closure with notices, SBOM, no Pro paths/types/implementations and no Dyad production telemetry/branding. Preserve applicable AG1/Dyad/third-party license texts. Treat bundled GPU/model payloads as separately reviewed material if later explicitly requested.

Run deterministic quality/retrieval/context/failure suites, then sequential live TS/Rust evaluation with the same locked model/backend/config/fixtures/cache conditions for every configured model. Use existing long-horizon, repository-context, context-intelligence and speed-probe scenarios; adapt their driver only, retaining verifier/evidence criteria. Record draft acceptance where exposed, not inferred. No simultaneous competing generation against the local endpoint.

Run both 8h and 24h repeated-session workloads with 2–3M externalized tokens, source edits, branch/fork/resume, window detach/reconnect, model/host crashes, corrupt-state recovery and preserved evidence. Fit post-warmup memory growth; distinguish expected persistent evidence growth from heap leak. Verify required recall, forbidden avoidance, contradiction handling, task success and zero stale-authoritative claims. Do not mark a shorter run as a duration pass.

Use disposable project/state destinations for source edits, corruption and crash injection. Preserve the original Cruzible/Dyad pins, the accepted migrated user state and all P13 archives. Disposable TS/Rust evaluation still requires the shared exclusive runtime lease; a separate workspace or state directory does not permit competing generation. Identify available OS runners, signing facilities, disk capacity and workload duration before starting long runs, recording unavailable prerequisites as pending gates. P13's specific authorized service stops do not authorize stopping a future externally owned process.

Test packaged Windows/macOS/Linux shell/process/protocol/preview behavior using available supported automation plus recorded manual cases where needed. Tuned local model support initially applies to the inspected Linux builds; other OS model-runtime compatibility remains unavailable until separately authorized matching builds exist. Separate shell readiness from local-inference support, and explicitly retain pending OS runners instead of claiming cross-platform completion.

## Tests and commands

Extend the gate/eval drivers to perform actual packaging, artifact inspection and matched workloads:

At the P13 completion commit, the `P14` gate and `release-differential` oracle suite below are not implemented. Implement their evidence validation and execution paths before using these commands as acceptance; the existing refusal is not a completed test. Route all new outputs to P14 without refreshing prior phase manifests.

```sh
cargo test --manifest-path product/Cargo.toml --workspace
npm --prefix product/apps/desktop run build
python3 tools/reference_oracles.py --suite release-differential --offline
python3 tools/phase_gate.py P14
```

The P14 gate must reference actual 8h/24h/live/platform report inputs with fingerprints and durations; it cannot synthesize success from their absence. Produce exact per-OS package/signing validation commands in the report; credentials/signing availability is a separate named gate. Test update interrupted mid-install, preserved external runtime paths, startup after crash, unknown pending operation and rollback archive readability.

## Performance and completion

Close matched packaged baselines: ≥30% lower shell idle memory versus Dyad, ≥50% smaller shell installation, idle CPU <0.5% of one core, startup p95 <1.5s excluding model load, display lag p95 <50ms, no >10% equivalent core p95 regression after noise analysis, bounded paused-subscriber queues and zero leaked owned children. Include all host/worker/webview processes; itemize optional models/browser/project runtimes and report combined totals. Backend tuning changes cannot count as a Rust optimization.

P13 measured native resume at 215,545 ms including full material verification, and a final preservation/authority audit at 540,581 ms. Its 6 ms renderer result measures DOM settlement after thaw only. Report shell readiness, preservation/admission, model load and usable-session readiness separately; the renderer result is not a startup benchmark. Investigate verification cost without skipping mandatory identity checks or silently changing the startup metric. P13 exact-output canaries used matched test instructions and transport controls; preserve their strict validators, and capture equivalent assembled prompts/context/cache conditions for broader P14 quality comparisons rather than assuming full application context is byte-identical.

Exit requires every parent/child capability disposition and invariant backed by actual evidence, all four defect regressions, all eight llama requirements, clean provenance and accepted platform/long-duration metrics. Failed quality/performance or unavailable required live checks mean incomplete acceptance, not a smaller silent definition of done. List scoped platform availability honestly.

## Rollback and handoff

Retain P13 export/checkpoint and immutable reference reports. If release gates fail, keep the last proven owner or perform tested rollback; do not rewrite old data in place. Deliver packages, SBOM/notices, runtime manifest references, compatibility matrix, raw measurements and release-candidate report for review. Stop before publishing unless separately instructed.

P13 rehearsed actual TS authority rollback on fixtures. Production recovery resumed a disposable, network-disabled copy of a new TS candidate, then cancelled rollback and retained native authority. That checkpoint predates the final desktop prompt. Any subsequent real rollback must freeze current native progress and export to a fresh TS destination; never activate the old candidate or original stale TS state. Unsupported native execution/checkpoint metadata must block downgrade while preserving both archives. Agent-state rollback does not reverse source edits or external effects.
