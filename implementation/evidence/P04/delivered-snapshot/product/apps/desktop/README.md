# AG1 desktop — P04 development bridge

The React/Tauri desktop runs the existing AG1 SDK through a supervised TypeScript worker. Rust owns native files, process lifecycle, project catalog and appearance preferences. The SDK remains the single authority for agent execution, sessions, tasks, memory, artifacts, approvals and model requests. One workspace executes at a time; previous tabs show bounded snapshots. This is the intermediate P04 desktop, before Rust authority migration.

The three model profiles reuse the original tuned llama.cpp binaries, libraries, weights and launch settings. The bridge checks their recorded identities before model actions, rejects drift and foreign endpoint ownership, and disables automatic tuning. Selecting a model does not load it. Sending a message in normal mode may start the selected preserved runtime. Fixture mode uses the actual SDK with a deterministic provider and makes no model request.

## Build, verify and run

Run from `/home/alex/Projects/AG1`, after P01–P03 have passed:

```sh
export PATH="$PWD/.local/toolchains/node-v24.14.0-linux-x64/bin:$PATH"
python3 tools/build_bridge.py
python3 tools/build_bridge_runtime.py
npm --prefix product/apps/desktop ci
npm --prefix product/apps/desktop run build
cargo build --manifest-path product/Cargo.toml -p ag1-desktop -p ag1-platform --release
python3 tools/package_bridge.py
.local/p04-package/run-ag1
```

The staged Linux launcher uses X11 with `WEBKIT_DISABLE_DMABUF_RENDERER=1`, required on the validated host. System GTK3/WebKitGTK 4.1, the pinned Node 24.14.0 toolchain and the separate verified `reference/ag1-bridge` dependency tree remain external dependencies. This staging is not a standalone installer. It does not copy model weights or tuned runtimes. Native Wayland, Windows and macOS need later validation.

`python3 tools/phase_gate.py P04` runs the phase checks and stages a production-feature binary. It requires previously captured native evidence and never launches a desktop itself. `implementation/evidence/P04/completion.json` is the completion authority. The packaged binary excludes the diagnostic visual broker.

## State and recovery

Normal development state lives under `.local/desktop`; `AG1_DESKTOP_STATE` can select another private destination. The host takes an exclusive writer lock. Workspace sessions, ledger, artifacts, memory and per-launch worker logs live below this destination. Original AG1 state and protected reference/runtime trees are not executing workspaces.

Commands are serialized against an authoritative revision. Read-only queries do not invalidate mutations. Command IDs and durable intent/results prevent replay; interrupted work is recorded instead of automatically rerun. Approval bindings cover the exact request and session scope. Reconnecting restores pending approval; restarting a dead worker does not approve it. Large requests require sequential full review before Allow is enabled.

Use Stop before changing an executing session, or Quit to drain and stop the owned worker. Preserve private state and worker logs when investigating failures. Rollback means retaining that state and using supported export through the immutable reference; never copy these journals over original user state. P05–P10 can implement Rust services independently while this worker remains authoritative. P13 owns the eventual authority cutover.

## Bounds and preview

Transport frames are capped at 64 KiB, queued output at 1 MiB and admitted commands at 256. Recent message projections are bounded to 20 messages, with full content stored as evidence; the renderer retains at most 32 snapshot tabs. Monaco loads lazily. Native file saves compare the byte revision and preserve external changes on conflict.

Static preview displays sanitized HTML in an opaque sandbox with no scripts, external resources, navigation, Node or agent IPC. The plain “Isolated preview fixture” is a security test, not a finished website preview. Interactive web/native previews and broader history/mode interfaces remain later phase work.

## Native evidence reproduction

The private diagnostic build uses `--features visual-test` and requires `AG1_FIXTURE_PROVIDER=1` plus a mode-0600 Unix broker socket. Notify the user before launching or relaunching a desktop. `tools/probes/launch_fixture_desktop.py` verifies and restarts only its recorded owned fixture process. The probes under `tools/probes` exercise settings, reconnect, approval paging, preview isolation, renderer pauses and matched screenshots in disposable workspaces.

The pinned Dyad comparison uses its supported E2E package, separate profile and private controller. Its editor requires the exact Monaco 0.52.2 CDN prefix; other external HTTP stays blocked. `dyad_dirty_capture.py` requires a loaded README editor and checks actual pane geometry and unsaved state. It never edits the reference source. `capture_visual_report.mjs` renders the retained screenshots into a headless review sheet.

See the phase report for measured process trees, visual differences, import/license closure and explicit remaining gates.
