# P12 — Native isolated clients and UI parity

P12 delivers an explicitly selected Rust desktop binding, native print/JSONL executable and standalone protocol SDK. Ordinary desktop authority remains TypeScript. Completion is certified only by this directory's `completion.json` with status `passed`; command logs, source fingerprints, scenario results and immutable delivered snapshot are part of that certificate.

The entry checkpoint was `ef68003` (P11). P11 artifacts and current original/runtime fingerprints were verified before implementation. The P11 handoff and all earlier evidence remain unchanged. The user authorized finishing P12, committing it and stopping; this work does not authorize P13 cutover.

## Delivered behavior

`ag1_platform::client::Host` composes the existing native storage, session registry, context compiler, provider scheduler, tool/policy/resource services and P11 target runtime. It owns its driver independently of renderer subscriptions. Clients consume generated Rust protocol messages and issue scoped commands. No React agent decisions or hidden TypeScript parent AgentSession were added. The separate extension helper retains its P09/P10 trust and callback boundaries.

The desktop adds typed conversation blocks, bounded snapshots and full journal paging, task/context/memory/artifact/activity drawers, model conflicts and explicit context selection, plan/questions, session tree/fork/input restoration, HTML export, resource and package actions, extension UI/flags/shortcuts, target controls/problems/logs, constrained owned previews, and a lazy owned terminal. File and resource saves use native compare-and-swap; failures retain the edited buffer and late acknowledgements preserve newer typing. Image input validates actual selected-model vision, decoded bounds and MIME. Retained images can be paged, verified and previewed. Bounded progress is advisory; completion comes from durable native receipts.

Files, export, uploads and project/terminal mutations use distinct durable client-effect receipts. Duplicate delivery observes the original result; an uncertain receipt cannot replay the effect. Recovery actions expose the recorded uncertainty. Session publication is resnapshotted across transient fences, preserving scope and revision consistency. Native jobs and project/terminal log drains continue with panels closed. Stop and shutdown address owned handles only.

`product/clients/README.md` documents the isolated configuration and CLI modes. `product/clients/sdk/README.md` documents the built ESM/declaration facade and embedding transport. Print uses native completion policy and full assistant text. JSONL retains bounded framing, correlation, clean stdout, cancellation and EOF settlement. The SDK can import without React, Tauri or the old TS agent loop.

## Compatibility and attribution

`product/clients/COMPATIBILITY.md` and `fixtures/clients/dispositions.json` account for all A01–A85, including explicit boundaries. A79 validates the reference JSON workflow and reproduces textual steering; its independent DAG API remains reference-only. A80 exercises the retained stdio client against owned child fixtures; it is unintegrated and has no HTTP/default MCP claim. A81's removed runner stays removed. A82–A84 remain independent Lit/Slack/pods reference clients. No Slack, SSH, cloud or deployment action was performed. A85 tests actual absent source/dist hook targets and the facade's explicit UnsupportedAPI errors.

Dynamic extension provider rebind and UI/provider callbacks inside native lifecycle hooks retain P10's DenyBridge boundary. Explicit admitted extension commands can use native UI and, with a separate host grant, the shared native inference scheduler and credential resolver. Credentials remain host bindings; there is no implicit migration of the user's vault or profiles. P06 tuning/owned llama lifecycle APIs are preserved; ordinary live endpoint ownership transfer remains P13. Native window embedding and judge-only vision retain the P11 unavailable disposition.

AG1 MIT notices are retained, including HTML export and SDK packaging. No new Dyad source, font, icon package, remote asset or UI dependency was introduced. The actual emitted renderer, TypeScript dependency closure, lazy Monaco worker, CSS and local fonts are audited with retained notices in `renderer-bundle-audit.json`. `provenance.json` fingerprints source action/API references and delivered destinations. Native dependency notices track the final Cargo lockfile.

## Acceptance and measurements

`python3 tools/phase_gate.py P12` runs formatting, warnings-as-errors Clippy, generated protocol verification, native core/storage/protocol/platform/client contracts, desktop contracts, component tests, SDK build/import tests, pinned reference stdio/disposition tests, actual WebKit malicious-preview denial, the renderer bundle audit and native visual scenarios. Gate validator tests reject missing/skipped coverage, unmatched or missing screenshots, absent freeze proof and p95 display latency at or above 50 ms. Exact cases and counts are in `passed-cases.json`, `coverage.json` and `completion.json`.

The native visual run uses an owned fake loopback provider and private authenticated Xvfb, with the real Tauri/WebKit application at 1440 × 900, DPR 1, packaged Geist fonts and light/dark themes. It verifies 1,000-message virtualization, actual Monaco dirty/CAS conflict behavior, native plan review, model conflict display, resource approval, terminal controls, provider failure/retry, compaction, a new empty session, streaming latency and completion while the renderer is frozen for 30 seconds. `native-visual.json` records measured latency and independent provider completion. `visual-comparison.html` places matched P04/P12 captures side by side. The added native status strip and typed evidence affordances are intentional; fixture content differs and pixel identity is not claimed.

Native retry/compaction, recovery and durability failure semantics are exercised by the current P10/P11 regression suites and P12 client receipt tests. Real compiler/web/native-window and browser contracts execute again against the current source. Measurements are Linux development-fixture observations, not live model quality or speed gains. Windows/macOS automation, live per-model ownership/quality/performance, release packaging and 8h/24h workloads remain the explicit P13/P14 gates.

## Preservation, rollback and next phase

The full gate checks original pins, tuned binary/library hashes, all 30 weight files and loader material before and after acceptance. The profile SHA-256 remains `aa02688c4d39ef73234f0ec381eed22dbe02463881626423049b75b5a12ad0d0`. No model was started, benchmarked, retuned, rebuilt, upgraded or substituted. No original user workspace, runtime configuration or weight file was written.

Rollback selects the unchanged ordinary TS bridge at startup after owned native work has quiesced. Keep isolated native journals, operation receipts, artifacts and uncertain recovery evidence; do not overwrite user state or attach a second owner to a live endpoint. `implementation/P13-HANDOFF.md` gives the explicit next-phase switch and verification commands. Stop after the P12 commit.
