# P12 — Complete AG1 surfaces and client compatibility

## Objective and reading

Finish the Dyad-derived UI and non-GUI client adapters against the Rust runtime in isolated mode. Read common instructions, UI_REUSE_MAP, LICENSE_AUDIT, interactive/print/RPC actions and API exports, adjacent/partial capability rows, and L01–L08. Scope A01,A26,A72–A75,A79–A85 plus frontend integration of all active core capabilities; I01,I20,I22,I23,I26–I28,I30.

## Entry and allowed work

Require P11 runtime targets, P10 complete Rust harness and P04 licensed shell. Work in desktop frontend/host, `product/clients/cli`, thin SDK compatibility facade, export adapters and fixtures. A small ag1-cli composition package is explicitly allowed by DECISIONS D01. Do not move agent decisions into React or keep a hidden TS loop in the SDK facade.

## Work and authority

Bind projects/sessions/chat/input/attachments/files/diffs/history/settings/approvals/notifications to Rust DTO view models. Preserve keyboard/scroll/focus/resize/theme, file save queue/CAS and Monaco disposal. Query stores are projections; local atoms hold presentation choices only. Model picker displays per-model runtime selection and configuration conflict instead of substituting backends. Changing model/context is a core command; tuning is a distinct explicit operation, never a settings mount effect.

Add a compact context/run/model status strip and Task, Context, Artifacts & memory, Activity drawer. Show original objective/latest instruction, evidence types, fresh/stale reasons, exact source revisions, budget versus externalized size, artifact byte paging, leases, jobs/subagents, queue/retry/recovery, model throughput and logs. Avoid fabricated completion percentages or generic verification badges. Runtime pane derives controls from web/native capabilities and shows actual diagnostics.

Translate mode-owned slash/action behavior into core commands and React interactions: plan/questions, queueing, model/context, session fork/tree/export, skills/templates/packages/extensions, diagnostics and workflow text steering. Provide CLI argument/config/print/JSONL mode adapters and a documented SDK facade over protocol. Preserve stdout cleanliness, response correlation, cancellation and mode completion policy. Prove core can run without React/Tauri. Keep old TUI as reference presentation; do not port terminal widget code into core.

Finalize extension feature/compatibility table from P09. A79 retains textual workflow and documented unintegrated DAG reference; A80 retains unintegrated stdio reference without invented HTTP/default MCP; A81 stays removed; A82/A83/A84 preserve independent Lit/Slack/pods reference clients and publish honest Rust API compatibility status. A85 verifies actual exported hooks/globals before promising a shim, with explicit unsupported errors for dead exports. None is silently advertised as migrated desktop functionality.

At the planning pin, the advertised `./hooks` source/dist targets are absent (DECISIONS D05). Test that disposition explicitly; do not confuse it with active extension hooks or create a new hooks API under a parity claim.

## Interfaces, reuse and tests

Copy only ledger-approved primitives/layouts; new AG1 message renderer uses typed blocks, not Dyad execution tags/Pro types. Themes/fonts/icons/workers have retained notices and local packaging. Native dialogs/clipboard/open URL are narrow host adapters. HTML export retains AG1/vendor notices and bounded redacted diagnostics.

Create ag1-cli `client_contract`, desktop component tests and packaged integration scenarios. Compare the same domain trace through CLI/JSONL/desktop; verify reload/closed panel does not cancel a core task, stale approval denied, late native-theme bootstrap ignored, failed save retains dirty buffer, artifact tamper visible and model conflict never retunes. Test loading/empty/streaming/retry/compaction/persistence failure/long-list states and malicious preview invoke attempts.

```sh
cargo test --manifest-path product/Cargo.toml -p ag1-cli --test client_contract
npm --prefix product/apps/desktop run typecheck
npm --prefix product/apps/desktop test -- --run
python3 tools/phase_gate.py P12
```

## Measurement and completion

Compare approved P04 visuals at matched viewport/theme/font scale, 1,000-message scroll, lazy editor/terminal, bounded logs and 30-second frozen renderer with resnapshot. Gate p95 streaming display lag <50ms under controlled load; record native platform automation limitations. Exit requires complete mapped actions, tested non-GUI client, audited bundle and compatibility dispositions for all A01–A85. Rust is usable in isolated workspaces; normal TS desktop authority has not yet moved.

## Rollback and handoff

Keep the known-good TS bridge selectable only at a quiescent workspace boundary. Revert UI/adapter changes without altering agent state. P13 receives a complete product, actual client compatibility evidence and one explicit cutover switch—not half-implemented pages or core APIs.
