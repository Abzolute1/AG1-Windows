# P04 — Real AG1 desktop through a guarded TS bridge

## Objective and reading

Deliver an early usable Dyad-derived desktop backed by real AG1 behavior. Read common instructions, DYAD_ARCHITECTURE, UI_REUSE_MAP, LICENSE_AUDIT, SDK/RPC/mode addenda and completion paths, B01–B04, and all L01–L08. Scope early A01–A31/A49–A75 presentation through the reference; I01,I20–I23,I26,I30. This is intermediate bridge parity, not completed Rust parity.

## Entry and allowed work

Require P03 process/workspace services, P02 protocol/fake host, P01 reference/runtime lock/provenance ledger. Create Tauri host and React app under `product/apps/desktop`, guarded worker under `product/compat/agent-worker`, explicit bridge patches against a separate reference copy, and component/E2E fixtures. Original reference stays immutable. No updates to any tuned runtime or original profile.

## Work and authority

Supervise one TS worker with the existing SDK/runtime, mode addenda, completion/approval policy and scheduler. Allow one executing workspace session initially; other tabs are snapshot viewers. Worker persists only in a new private destination. App preferences and project catalog are host-owned separate state; session/task/memory/artifacts remain TS-owned. Host controls worker lifecycle; TS controls its model requests. Rust must not acquire a second scheduler permit for each TS generation.

Translate SDK/RPC events into the P02 protocol, adding real snapshot/evidence/task/runtime access and durable-commit acknowledgments. Worker command IDs/intent results and reconnect metadata need a serialized, tested ledger in its private state; transient token events alone are not durable evidence. Drain session work before snapshot/resume/replacement. Capture and restore approval/question state on reconnect.

Apply narrow reviewed bridge patches for B01 aggregate output, B02 corrupt memory handling, B03 one-snapshot provenance and B04 fork artifact closure, and D01 process identity protections. Reuse corrected oracle tests from P01; preserve old reference as evidence. Guard the llama resolver against binary/profile drift and foreign-port eviction, disable unsolicited tuning, and retain model-specific binaries/libraries/flags exactly. A mismatched or unknown running process yields an explicit observation/conflict, not automatic replacement. Retain opt-in tuning API only as an explicitly requested later action.

Build home/project selection, chat/input/streaming, files with pending-save/conflict, diff, settings/model status and approvals using individually approved Dyad primitives/layouts. Source candidates: globals.css, ui primitives, pages/chat, ChatPanel/chat tabs/list/input and preview_panel FileTree/FileEditor/FileDiffEditor. Decouple every import from Dyad IPC/state/Pro. Keep layout geometry/themes/keyboard, lazy Monaco and virtualized messages. No cloud/billing or Pro annotation.

Perform an early isolated web-preview feasibility experiment: no agent invoke permissions, credentials or Node API; test origin/navigation/focus/overlay behavior. If embedded preview fails on a platform, record external-browser display capability and the P11 decision gate. Do not block core work by inventing CDP support.

## Interfaces, tests and commands

Create frontend `test`, `typecheck`, `build` scripts and ag1-desktop `bridge_contract`. Test real worker prompt→tool→continuation with fake provider, cancel/drain, resume/fork, queued steering, denied/stale approvals, persistence failure, worker death, renderer reload and all four corrected defect cases. Verify three profile selections and zero tune/rebuild calls. Fake server/model tests must not contact the user's endpoint.

```sh
cargo test --manifest-path product/Cargo.toml -p ag1-desktop --test bridge_contract
npm --prefix product/apps/desktop run typecheck
npm --prefix product/apps/desktop test -- --run
python3 tools/phase_gate.py P04
```

Only tests created for this phase run under its gate. Capture pinned Dyad and bridge screenshots for home/chat/dirty files/diff/settings/dialog/loading/error in both themes, matched viewport/scale. Review AG1 differences. Scan transitive code/type/CSS/worker/asset imports and actual bundle, preserving LICENSE/NOTICE/font/icon obligations. Pro dependency search alone is insufficient.

## Measurement and completion

Use compatible pinned Node for Dyad benchmark and a real desktop/display. Record packaged startup, five-minute idle memory/CPU/process tree, Monaco/preview and paused-renderer load for Dyad and bridge; include the TS worker. Record unsupported OS runner/visual gaps. Require bounded queues, reconnect correctness, usable development-platform desktop and all bridge safeguards before inviting real tasks. Numerical final shell savings can remain unmet by the temporary bridge but must be reported.

## Rollback and handoff

Stop/drain the owned worker; retain its private state/archive; return to immutable reference using supported export only. Existing external model processes/config remain unchanged. P05–P10 can build Rust services in shadow while this usable desktop stays TS-authoritative. P11 receives actual preview feasibility evidence, not an assumed Electron replacement.
